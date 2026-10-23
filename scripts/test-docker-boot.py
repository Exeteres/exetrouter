#!/usr/bin/env python3
"""Test Docker boot recovery with isolated systemd units; requires Linux and root."""
import configparser
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def systemctl(*args, check=True):
    return subprocess.run(
        ['systemctl', *args], check=check, capture_output=True, text=True, timeout=15,
    ).stdout.strip()


def replace_directives(text, values):
    return '\n'.join(
        f'{key}={values[key]}' if '=' in line and (key := line.split('=', 1)[0]) in values
        else line for line in text.splitlines()
    ) + '\n'


class BootRecovery(unittest.TestCase):
    def run_scenario(self, failures):
        main = (ROOT / 'deploy/docker/exetrouter-compose.service').read_text()
        gateway = (ROOT / 'deploy/systemd/exetrouter-sshd.service').read_text()
        config = configparser.ConfigParser(interpolation=None)
        config.read_string(main)
        burst = config.getint('Unit', 'StartLimitBurst')
        self.assertEqual(burst, 3)
        self.assertEqual(config.getint('Unit', 'StartLimitIntervalSec'), 300)
        self.assertEqual(config.getint('Service', 'RestartSec'), 10)
        with tempfile.TemporaryDirectory(prefix='exetrouter-boot-test-', dir='/run') as temporary:
            work = Path(temporary)
            name = work.name
            units = [f'{name}.service', f'{name}-sshd.service']
            paths = [Path('/run/systemd/system') / unit for unit in units]
            created = []
            attempt = work / 'attempt.py'
            attempt.write_text(
                'import sys, time\nfrom pathlib import Path\n'
                'p = Path(__file__).with_name("attempts")\n'
                'n = int(p.read_text()) + 1 if p.exists() else 1\n'
                'p.write_text(str(n))\n'
                'with p.with_name("times").open("a") as f: f.write(str(time.monotonic()) + "\\n")\n'
                'sys.exit(1 if n <= int(sys.argv[1]) else 0)\n'
            )
            ready = work / 'ready.py'
            ready.write_text(
                'import subprocess, sys\nfrom pathlib import Path\n'
                'state = subprocess.check_output(["systemctl", "show", sys.argv[1], '
                '"--property=ActiveState", "--value"], text=True).strip()\n'
                'assert state == "active", state\n'
                'Path(__file__).with_name("gateway-started").touch()\n'
            )
            # Keep the real retry/dependency rules, but replace all application work.
            main = main.replace('exetrouter-sshd.service', units[1])
            main = replace_directives(main, {
                'Requires': '', 'After': '',
                'Wants': ' '.join(word for word in config['Unit']['Wants'].split()
                                  if word != 'network-online.target').replace(
                                      'exetrouter-sshd.service', units[1]),
                'WorkingDirectory': str(work), 'ExecStartPre': '/usr/bin/true',
                'ExecStart': f'/usr/bin/python3 {attempt} {failures}',
                'ExecStop': '/usr/bin/true', 'ReadWritePaths': str(work),
            })
            gateway = gateway.replace('exetrouter.service', units[0])
            gateway = replace_directives(gateway, {
                'After': units[0],
                'ExecStart': f'/usr/bin/sleep infinity\n'
                             f'ExecStartPre=/usr/bin/python3 {ready} {units[0]}',
            })
            for production in ('exetrouter.service', 'exetrouter-sshd.service', '/usr/bin/docker'):
                self.assertNotIn(production, main + gateway, 'test units must stay isolated')
            try:
                for path, content in zip(paths, (main, gateway)):
                    with path.open('x') as output:
                        created.append(path)
                        output.write(content)
                systemctl('daemon-reload')
                # Starting the gateway reproduces its boot-time dependency on the API.
                systemctl('start', '--no-block', units[1])
                deadline = time.monotonic() + 75
                while time.monotonic() < deadline:
                    state = dict(line.split('=', 1) for line in systemctl(
                        'show', units[0], '--property=Result,ActiveState,NRestarts',
                    ).splitlines())
                    result = state['Result']
                    active = systemctl('show', units[1], '--property=ActiveState', '--value')
                    if (failures < burst and active == 'active') or (
                        state['ActiveState'] == 'failed' and int(state['NRestarts']) >= burst
                    ):
                        break
                    time.sleep(0.2)
                else:
                    state = systemctl('show', units[0], '--property=ActiveState,SubState,Result')
                    self.fail(f'boot recovery exceeded 75 seconds: {state}')
                count = int((work / 'attempts').read_text())
                times = [float(value) for value in (work / 'times').read_text().splitlines()]
                self.assertTrue(all(later - earlier >= 9.5 for earlier, later in zip(times, times[1:])),
                                'failed startup must wait ten seconds before retrying')
                started = (work / 'gateway-started').exists()
                if failures < burst:
                    self.assertEqual(count, failures + 1)
                    self.assertEqual(active, 'active')
                    self.assertEqual(result, 'success')
                    self.assertTrue(started, 'gateway must start only after the API is active')
                else:
                    self.assertEqual(count, burst)
                    self.assertEqual(state['ActiveState'], 'failed')
                    self.assertEqual(int(state['NRestarts']), burst)
                    self.assertIn(result, ('exit-code', 'start-limit-hit'))
                    self.assertNotEqual(active, 'active')
                    self.assertFalse(started, 'gateway must not start after a failed API startup')
            finally:
                systemctl('stop', *reversed(units), check=False)
                systemctl('reset-failed', *units, check=False)
                for path in created:
                    path.unlink(missing_ok=True)
                systemctl('daemon-reload')

    def test_transient_failure_recovers_api_and_gateway(self):
        self.run_scenario(failures=1)

    def test_healthy_start_runs_once_and_starts_gateway(self):
        self.run_scenario(failures=0)

    def test_persistent_failure_is_bounded_and_keeps_gateway_stopped(self):
        self.run_scenario(failures=100)


if __name__ == '__main__':
    if sys.platform != 'linux' or os.geteuid() != 0 or not Path('/run/systemd/system').is_dir():
        raise SystemExit('Run as root on a Linux systemd host; only isolated test units are used.')
    unittest.main()
