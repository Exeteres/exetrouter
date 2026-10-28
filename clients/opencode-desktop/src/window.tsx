import { For, Show, createSignal, onCleanup } from "solid-js"
import { Command, ComposerPill, MenuItem, Panel, SettingsPage, Style, createLatest, type Context, type Plugin } from "@opencode/plugin/gui"
import { Dashboard } from "./contract"
import { count, date, duration, freshness, percent } from "./format"
import { ExetRouter } from "./rpc"
import type { UsageInput } from "./schemas"
import css from "./window.css?inline"

function Overview(props: { ctx: Context }) {
  const bridge = props.ctx.main(Dashboard)
  const state = () => bridge.state()
  return <>
    <p class="exr-muted">Cached observations from the local exr profile. Refresh does not submit inference or redeem reset credits.</p>
    <Show when={state()?.error}><p role="alert" class="exr-warning">{state()?.error} Previous data, if shown, is historical.</p></Show>
    <Show when={state()?.snapshot} fallback={<p>No snapshot available. Configure exr, then refresh.</p>}>
      {(report) => <>
        <p><strong>{report().configured ? "Configured" : "Needs attention"}</strong> · ExetRouter {report().version}</p>
        <p class="exr-muted">Snapshot: {date(report().checkedAt)}. Public API readiness and inference are not checked.</p>
        <For each={report().accounts} fallback={<p>No account observations.</p>}>
          {(account) => <section class="exr-card">
            <h3>{account.label}</h3>
            <p class="exr-muted">{account.enabled === null ? "Availability unknown" : account.enabled ? "Enabled" : "Deactivated"}{account.locked ? " · Operator locked" : ""}</p>
            <For each={account.windows} fallback={<p>Quota unknown.</p>}>
              {(window) => <div class="exr-window">
                <strong>{duration(window.minutes)}: {percent(window.remaining)} remaining</strong>
                <span>{state()?.error ? "historical" : freshness(window)} · resets {date(window.resetAt)}</span>
                <span class="exr-muted">Observed {date(window.observedAt)}</span>
              </div>}
            </For>
          </section>}
        </For>
      </>}
    </Show>
  </>
}

function UsageView(props: { ctx: Context }) {
  const ui = props.ctx.ui
  const [period, setPeriod] = createSignal<UsageInput["period"]>("day")
  const [by, setBy] = createSignal<UsageInput["by"]>("model")
  const [revision, setRevision] = createSignal(0)
  const report = createLatest(() => ({ period: period(), by: by(), revision: revision() }), (input, signal) =>
    props.ctx.main(Dashboard).usage({ period: input.period, by: input.by }, { signal }))
  return <>
    <div class="exr-actions">
      <label>Period <select value={period()} onChange={(e) => setPeriod(e.currentTarget.value as UsageInput["period"])}>
        <option value="day">Today</option><option value="24h">Last 24 hours</option><option value="week">This week</option><option value="month">This month</option>
      </select></label>
      <label>Group <select value={by()} onChange={(e) => setBy(e.currentTarget.value as UsageInput["by"])}>
        <option value="model">Model</option><option value="user">User</option><option value="token">Token</option>
      </select></label>
      <ui.Button busy={report.loading} onClick={() => setRevision((n) => n + 1)}>Refresh usage</ui.Button>
    </div>
    <p class="exr-muted">Model/user reports are shared pool totals. Token reports contain only your tokens. Unknown consumption is never counted as zero.</p>
    <Show when={report.error}><p role="alert" class="exr-warning">Usage unavailable. Check the saved exr profile. Any previous report is historical.</p></Show>
    <Show when={report.loading && !report.latest}><ui.Loader /></Show>
    <Show when={report.loading && report.latest}><p class="exr-muted">Previous report shown while refreshing.</p></Show>
    <Show when={report.latest}>
      {(data) => <>
        <p class="exr-muted">{date(data().from)} – {date(data().to)} · {data().timezone}</p>
        <div class="exr-table"><table>
          <thead><tr><th>Group</th><th>Requests</th><th>Known input</th><th>Known output</th><th>Known total</th><th>Unknown / partial requests</th></tr></thead>
          <tbody><For each={data().rows} fallback={<tr><td colspan="6">No usage recorded.</td></tr>}>
            {(row) => <tr><td>{row.name}</td><td>{count(row.requests)}</td><td>{count(row.input)}</td><td>{count(row.output)}</td><td>{count(row.total)}</td><td>{count(row.unknown)} / {count(row.partial)}</td></tr>}
          </For></tbody>
        </table></div>
      </>}
    </Show>
  </>
}

function DashboardView(props: { ctx: Context; remote?: boolean }) {
  const ui = props.ctx.ui
  const [tab, setTab] = createSignal("overview")
  const bridge = props.ctx.main(Dashboard)
  const refresh = async () => {
    try { await bridge.refresh({ signal: props.ctx.signal }) }
    catch { props.ctx.toast.show({ variant: "error", title: "ExetRouter refresh unavailable" }) }
  }
  return <div class="exr">
    <div class="exr-actions"><h2>ExetRouter</h2><ui.Button busy={bridge.state()?.refreshing} onClick={() => void refresh()}>Refresh snapshot</ui.Button></div>
    <Show when={props.remote}><p class="exr-warning">This dashboard uses the desktop's local exr profile, not the remote OpenCode server's configuration.</p></Show>
    <div class="exr-actions" aria-label="ExetRouter views">
      <ui.Button variant={tab() === "overview" ? "contrast" : "ghost"} onClick={() => setTab("overview")}>Overview</ui.Button>
      <ui.Button variant={tab() === "usage" ? "contrast" : "ghost"} onClick={() => setTab("usage")}>Usage</ui.Button>
    </div>
    <Show when={tab() === "overview"} fallback={<UsageView ctx={props.ctx} />}><Overview ctx={props.ctx} /></Show>
  </div>
}

function Settings(props: { ctx: Context }) {
  const ui = props.ctx.ui
  const [revision, setRevision] = createSignal(0)
  const [busy, setBusy] = createSignal(false)
  const timer = setInterval(() => setRevision((n) => n + 1), 30_000)
  onCleanup(() => clearInterval(timer))
  const status = createLatest(() => ({ client: props.ctx.client, revision: revision() }), ({ client }, signal) => client.rpc(ExetRouter).status({}, { signal }))
  const refresh = async () => {
    setBusy(true)
    try { await props.ctx.client.rpc(ExetRouter).refresh({}, { signal: props.ctx.signal }); setRevision((n) => n + 1) }
    catch { props.ctx.toast.show({ variant: "error", title: "Catalog refresh unavailable" }) }
    finally { setBusy(false) }
  }
  const connect = async () => {
    let key = await props.ctx.dialogs.prompt({ title: "Connect an existing ExetRouter token", description: "The token is saved in OpenCode's credential store, not in plugin settings. Verify the API endpoint below first.", secret: true })
    if (!key) return
    try {
      await props.ctx.client.integration.connect.key({ integrationID: "exetrouter", key })
      await refresh()
    } catch { props.ctx.toast.show({ variant: "error", title: "ExetRouter connection could not be saved" }) }
    finally { key = undefined }
  }
  return <div class="exr">
    <h2>ExetRouter</h2>
    <p>Models use the native OpenAI Responses provider, WebSocket or HTTP, store=false and native compaction. The plugin does not intercept prompts or responses.</p>
    <Show when={status.latest} fallback={<p>Catalog status unavailable.</p>}>
      {(data) => <section class="exr-card"><p>API endpoint: <code>{data().baseURL}</code></p><p>Catalog: {data().state} · {data().models} models</p><p>Last successful fetch: {date(data().fetchedAt === null ? null : data().fetchedAt! / 1000)}</p></section>}
    </Show>
    <div class="exr-actions"><ui.Button onClick={() => void connect()}>Connect token</ui.Button><ui.Button busy={busy()} onClick={() => void refresh()}>Refresh models</ui.Button></div>
    <p class="exr-muted">Alternatively, use OpenCode Accounts to connect ExetRouter, or supply EXETROUTER_TOKEN to the OpenCode server. A terminal's environment is not automatically inherited by Desktop.</p>
    <h3>Connection settings</h3>
    <p>Edit this plugin's options in the global opencode.json(c): baseURL, transport, exrBinary, exrConfig and refreshSeconds. Changes reload the plugin. Never place a token in options.</p>
    <p>Dashboard executable: <code>{String(props.ctx.options.exrBinary ?? "exr")}</code></p>
    <p>Dashboard profile: <code>{String(props.ctx.options.exrConfig ?? "Default exr profile")}</code></p>
    <p>The HTTP endpoint and SSH profile are configured separately; ensure they refer to the intended router. Standalone requires an already running exr or exr serve. This MVP does not change profiles, start services, issue tokens or manage OAuth.</p>
    <p class="exr-muted">On a remote OpenCode server, install/configure the server part there as well. Local Desktop credentials are not forwarded to it.</p>
    <DashboardView ctx={props.ctx} />
  </div>
}

export const setup: Plugin.Setup = (ctx) => {
  ctx.add(Style, css)
  const open = () => {
    const session = ctx.sessions.current()
    if (session) ctx.layout.open(`${ctx.id}:dashboard`, session)
    else ctx.layout.settings(ctx.id)
  }
  ctx.add(Command, { id: "open", title: "Open ExetRouter", keywords: "router quota usage limits", run: open })
  ctx.add(MenuItem, (): MenuItem => ({ menu: "session.panel", id: "open", title: "ExetRouter", icon: "server", run: open }))
  const tab = { id: "dashboard", title: "ExetRouter", closable: "compact" as const, fallback: true }
  ctx.add(Panel, {
    id: "dashboard", region: "side",
    list: (input) => input.open.includes("dashboard") ? [tab] : [],
    render: (props) => <DashboardView ctx={ctx} remote={!props.session.server.builtin} />,
  })
  ctx.add(SettingsPage, { id: ctx.id, title: "ExetRouter", icon: "server", render: () => <Settings ctx={ctx} /> })
  const bridge = ctx.main(Dashboard)
  ctx.add(ComposerPill, {
    id: "status",
    show: (input) => {
      if (!input.session.server.builtin || input.screen.composer.model()?.providerID !== "exetrouter") return undefined
      const state = bridge.state()
      const report = state?.snapshot
      return {
        label: report ? `ExetRouter · ${report.accounts.length} accounts` : "ExetRouter · unavailable",
        title: "Local exr profile snapshot; not verified against this session's router or upstream account",
        icon: "server",
        tone: state?.error || !report?.configured ? "warning" : undefined,
        run: open,
      }
    },
  })
}
