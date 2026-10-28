//! A deliberately small, dual-era MCP Streamable HTTP binding. Tools use the
//! same native operations as the REST API; no loopback HTTP, shell or replay.
use super::*;
use axum::{
    body::{Body, Bytes},
    extract::FromRequest,
};
use futures_util::StreamExt;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::{collections::HashMap, convert::Infallible, pin::Pin, sync::Mutex};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

const MODERN: &str = "2026-07-28";
const LEGACY: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];
const INSTRUCTIONS: &str = "Use generate_image or edit_image when asked to create or edit images. They use ChatGPT image generation and return image content, not a text approximation. Calls consume image quota. Do not automatically retry an interrupted call: its outcome may be unknown. The server cannot read client-local files.";

pub(super) struct McpState {
    slots: limits::Slots,
    active: Mutex<HashMap<[u8; 32], CancellationToken>>,
}
impl McpState {
    pub(super) fn new(per_user: usize) -> Self {
        Self {
            slots: limits::Slots::new(per_user),
            active: Mutex::new(HashMap::new()),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Tool {
    Generate,
    Edit,
}
const TOOLS: &[Tool] = &[Tool::Generate, Tool::Edit];
impl Tool {
    fn read_only(self) -> bool {
        false
    }
    fn name(self) -> &'static str {
        match self {
            Self::Generate => "generate_image",
            Self::Edit => "edit_image",
        }
    }
    fn path(self) -> &'static str {
        match self {
            Self::Generate => "images/generations",
            Self::Edit => "images/edits",
        }
    }
    fn surface(self) -> &'static str {
        match self {
            Self::Generate => "mcp/generate_image",
            Self::Edit => "mcp/edit_image",
        }
    }
    fn definition(self) -> Value {
        let mut schema = json!({"type":"object","additionalProperties":false,"properties":{
            "prompt":{"type":"string","minLength":1,"description":"Describe the image to create, or the changes to make."},
            "quality":{"type":"string","enum":["auto","low","medium","high"]},
            "size":{"type":"string","minLength":1,"description":"Image dimensions supported by ChatGPT, or auto (default)."},
            "background":{"type":"string","enum":["auto","transparent","opaque"]}
        },"required":["prompt"]});
        if self == Self::Edit {
            schema["properties"]["image_urls"] = json!({"type":"array","minItems":1,"maxItems":5,"items":{"type":"string","pattern":"^(data:image/|https://)"},"description":"One to five inline data:image/...;base64,... or HTTPS image URLs. Client-local paths and backend file IDs are not supported."});
            schema["required"]
                .as_array_mut()
                .expect("schema")
                .push(json!("image_urls"));
        }
        json!({"name":self.name(),"title":if self==Self::Generate {"Generate image"} else {"Edit image"},
            "description":if self==Self::Generate {"Generate one image using ChatGPT gpt-image-2. Returns an image content block for the client to display or save. Consumes image quota; not idempotent. Never automatically retry after interruption."} else {"Edit or combine up to five images using ChatGPT gpt-image-2 and a prompt. Returns an image content block. Supply inline or HTTPS references, not local paths. Consumes image quota; never automatically retry after interruption."},
            "inputSchema":schema,"annotations":{"readOnlyHint":self.read_only(),"destructiveHint":false,"idempotentHint":false,"openWorldHint":true}})
    }
    fn payload(self, args: &Value) -> Option<Value> {
        let fields = args.as_object()?;
        if fields.keys().any(|key| {
            !["prompt", "quality", "size", "background"].contains(&key.as_str())
                && !(self == Self::Edit && key == "image_urls")
        }) || !args["prompt"]
            .as_str()
            .is_some_and(|v| !v.trim().is_empty())
        {
            return None;
        }
        let mut body = json!({"model":"gpt-image-2","prompt":args["prompt"]});
        for (name, choices) in [
            ("quality", &["auto", "low", "medium", "high"][..]),
            ("background", &["auto", "transparent", "opaque"][..]),
        ] {
            if let Some(value) = fields.get(name) {
                if !value.as_str().is_some_and(|s| choices.contains(&s)) {
                    return None;
                }
                body[name] = value.clone();
            }
        }
        if let Some(size) = fields.get("size") {
            if !size.as_str().is_some_and(|s| !s.trim().is_empty()) {
                return None;
            }
            body["size"] = size.clone();
        }
        if self == Self::Edit {
            let images = args["image_urls"].as_array()?;
            if images.is_empty()
                || images.len() > 5
                || images.iter().any(|url| {
                    !url.as_str()
                        .is_some_and(|s| s.starts_with("data:image/") || s.starts_with("https://"))
                })
            {
                return None;
            }
            body["images"] = images.iter().map(|url| json!({"image_url":url})).collect();
        }
        Some(body)
    }

    async fn invoke(
        self,
        state: Arc<AppState>,
        identity: Identity,
        request_id: RequestId,
        id: Value,
        modern: bool,
        payload: Value,
    ) -> Response {
        match self {
            Self::Generate | Self::Edit => {
                let reply = ImageReply {
                    surface: self.surface(),
                    id,
                    modern,
                };
                native::mcp_image(state, identity, request_id, payload, self.path(), reply).await
            }
        }
    }
}

fn server_info() -> Value {
    json!({"name":"exetrouter","version":env!("CARGO_PKG_VERSION")})
}
fn capabilities() -> Value {
    json!({"tools":{"listChanged":false}})
}
fn result(id: Value, mut value: Value, modern: bool) -> Value {
    if modern {
        value["resultType"] = json!("complete");
        value["_meta"] = json!({"io.modelcontextprotocol/serverInfo":server_info()});
    }
    json!({"jsonrpc":"2.0","id":id,"result":value})
}
fn json_response(status: StatusCode, value: Value) -> Response {
    (status, [("cache-control", "no-store")], Json(value)).into_response()
}
fn rpc_error(id: Option<&Value>, status: StatusCode, code: i64, message: &'static str) -> Response {
    let mut value = json!({"jsonrpc":"2.0","error":{"code":code,"message":message}});
    if let Some(id) = id {
        value["id"] = id.clone();
    }
    json_response(status, value)
}
fn tool_error(id: Value, modern: bool, message: &'static str) -> Response {
    json_response(
        StatusCode::OK,
        result(
            id,
            json!({"isError":true,"content":[{"type":"text","text":message}]}),
            modern,
        ),
    )
}
fn valid_id(id: &Value) -> bool {
    id.is_i64() || id.is_u64() || id.as_str().is_some_and(|v| v.len() <= 2048)
}
fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut all = headers.get_all(name).iter();
    let first = all.next()?;
    if all.next().is_some() {
        return None;
    }
    first.to_str().ok()
}
fn accepts(headers: &HeaderMap) -> bool {
    let accepts = |mime: &str| {
        headers
            .get_all("accept")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .any(|v| {
                let mut parts = v.split(';');
                parts
                    .next()
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case(mime))
                    && !parts.any(|p| {
                        p.trim().strip_prefix("q=").is_some_and(|q| {
                            q.parse::<f64>()
                                .map_or(true, |q| !q.is_finite() || q <= 0.0)
                        })
                    })
            })
    };
    accepts("application/json") && accepts("text/event-stream")
}

// Legacy session handles retain no server-side session or credentials. They
// correlate cancellation IDs without allowing one client/user to cancel another.
fn mac(key: &[u8], user: i64, domain: &[u8], parts: &[&[u8]]) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC key");
    mac.update(b"exetrouter-mcp-v1\0");
    mac.update(&user.to_be_bytes());
    mac.update(domain);
    for part in parts {
        mac.update(&(part.len() as u64).to_be_bytes());
        mac.update(part);
    }
    mac
}
fn session(key: &[u8], user: i64, version: &str) -> String {
    use rand::RngCore;
    let mut nonce = [0; 32];
    rand::thread_rng().fill_bytes(&mut nonce);
    let nonce = hex::encode(nonce);
    let tag = mac(
        key,
        user,
        b"session",
        &[version.as_bytes(), nonce.as_bytes()],
    )
    .finalize()
    .into_bytes();
    format!("exrmcp1.{version}.{nonce}.{}", hex::encode(tag))
}
fn session_version<'a>(key: &[u8], user: i64, handle: &'a str) -> Option<&'a str> {
    let parts = handle.split('.').collect::<Vec<_>>();
    if parts.len() != 4
        || parts[0] != "exrmcp1"
        || !LEGACY.contains(&parts[1])
        || parts[2].len() != 64
        || parts[3].len() != 64
    {
        return None;
    }
    let tag = hex::decode(parts[3]).ok()?;
    mac(
        key,
        user,
        b"session",
        &[parts[1].as_bytes(), parts[2].as_bytes()],
    )
    .verify_slice(&tag)
    .ok()?;
    Some(parts[1])
}
fn request_key(key: &[u8], user: i64, session: &str, id: &Value) -> [u8; 32] {
    let id = serde_json::to_vec(id).expect("JSON id");
    mac(key, user, b"request", &[session.as_bytes(), &id])
        .finalize()
        .into_bytes()
        .into()
}
struct ActiveRequest {
    state: Arc<AppState>,
    key: [u8; 32],
}
impl Drop for ActiveRequest {
    fn drop(&mut self) {
        self.state
            .mcp
            .active
            .lock()
            .expect("MCP requests")
            .remove(&self.key);
    }
}

pub(super) async fn not_allowed(headers: HeaderMap) -> Response {
    if headers.contains_key("origin") {
        return rpc_error(
            None,
            StatusCode::FORBIDDEN,
            -32600,
            "Browser origins are not allowed",
        );
    }
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [("allow", "POST"), ("cache-control", "no-store")],
    )
        .into_response()
}

pub(super) async fn post(
    State(state): State<Arc<AppState>>,
    Extension(identity): Extension<Identity>,
    Extension(request_id): Extension<RequestId>,
    request: Request,
) -> Response {
    let headers = request.headers();
    if headers.get_all("authorization").iter().count() != 1 {
        return rpc_error(
            None,
            StatusCode::UNAUTHORIZED,
            -32600,
            "Exactly one bearer credential is required",
        );
    }
    // Native desktop/CLI clients do not need Origin. Reject every browser
    // origin rather than infer a public origin from untrusted proxy headers.
    if headers.contains_key("origin") {
        return rpc_error(
            None,
            StatusCode::FORBIDDEN,
            -32600,
            "Browser origins are not allowed",
        );
    }
    if !accepts(headers) {
        return rpc_error(
            None,
            StatusCode::NOT_ACCEPTABLE,
            -32600,
            "Accept must include application/json and text/event-stream",
        );
    }
    let headers = headers.clone();
    let payload = tokio::time::timeout(
        Duration::from_secs(10),
        Json::<Value>::from_request(request, &state),
    )
    .await;
    let body = match payload {
        Ok(Ok(Json(body))) => body,
        Ok(Err(JsonRejection::MissingJsonContentType(_))) => {
            return rpc_error(
                None,
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                -32600,
                "Content-Type must be application/json",
            )
        }
        Ok(Err(_)) => return rpc_error(None, StatusCode::BAD_REQUEST, -32700, "Invalid JSON"),
        Err(_) => {
            return rpc_error(
                None,
                StatusCode::REQUEST_TIMEOUT,
                -32600,
                "Request body read timed out",
            )
        }
    };
    let id = body.get("id").filter(|v| valid_id(v));
    if !body.is_object()
        || body["jsonrpc"] != "2.0"
        || body.get("id").is_some_and(|v| !valid_id(v))
        || !body["method"].is_string()
        || body.get("params").is_some_and(|v| !v.is_object())
        || body.get("result").is_some()
        || body.get("error").is_some()
    {
        return rpc_error(id, StatusCode::BAD_REQUEST, -32600, "Invalid request");
    }
    let method = body["method"].as_str().expect("validated");
    let params = &body["params"];
    if params.get("_meta").is_some_and(|v| !v.is_object())
        || params
            .pointer("/_meta/openai~1readOnly")
            .is_some_and(|v| !v.is_boolean())
    {
        return rpc_error(
            id,
            StatusCode::BAD_REQUEST,
            -32602,
            "Invalid request metadata",
        );
    }
    let supplied = header(&headers, "mcp-protocol-version");
    let meta_version = params
        .pointer("/_meta/io.modelcontextprotocol~1protocolVersion")
        .and_then(Value::as_str);
    let initializing = method == "initialize";
    if headers.contains_key("mcp-protocol-version") && supplied.is_none() {
        return rpc_error(
            id,
            StatusCode::BAD_REQUEST,
            -32020,
            "Invalid protocol version header",
        );
    }
    if !initializing
        && meta_version
            .zip(supplied)
            .is_some_and(|(body, header)| body != header)
    {
        return rpc_error(
            id,
            StatusCode::BAD_REQUEST,
            -32020,
            "Protocol version header/body mismatch",
        );
    }
    let requested = meta_version.or(supplied);
    if !initializing && requested.is_some_and(|v| v != MODERN && !LEGACY.contains(&v)) {
        let requested = requested.expect("checked");
        if requested.len() != 10
            || !requested.bytes().enumerate().all(|(index, byte)| {
                if index == 4 || index == 7 {
                    byte == b'-'
                } else {
                    byte.is_ascii_digit()
                }
            })
        {
            return rpc_error(
                id,
                StatusCode::BAD_REQUEST,
                -32602,
                "Invalid protocol version",
            );
        }
        let mut response = json!({"jsonrpc":"2.0","error":{"code":-32022,"message":"Unsupported protocol version","data":{"supported":[MODERN,"2025-11-25","2025-06-18","2025-03-26"],"requested":requested}}});
        if let Some(id) = id {
            response["id"] = id.clone();
        }
        return json_response(StatusCode::BAD_REQUEST, response);
    }
    let modern = !initializing
        && (supplied == Some(MODERN)
            || meta_version == Some(MODERN)
            || method == "server/discover");
    if modern {
        if supplied != Some(MODERN) || meta_version != Some(MODERN) {
            return rpc_error(
                id,
                StatusCode::BAD_REQUEST,
                -32020,
                "Protocol version header/body mismatch",
            );
        }
        if !params
            .pointer("/_meta/io.modelcontextprotocol~1clientCapabilities")
            .is_some_and(Value::is_object)
        {
            return rpc_error(
                id,
                StatusCode::BAD_REQUEST,
                -32602,
                "Missing or invalid client capabilities",
            );
        }
        let decode_name = |value: &str| -> Option<String> {
            use base64::{engine::general_purpose::STANDARD, Engine};
            if let Some(encoded) = value
                .strip_prefix("=?base64?")
                .and_then(|v| v.strip_suffix("?="))
            {
                String::from_utf8(STANDARD.decode(encoded).ok()?).ok()
            } else {
                Some(value.to_owned())
            }
        };
        if header(&headers, "mcp-method") != Some(method)
            || (method == "tools/call"
                && (header(&headers, "mcp-name").is_none()
                    || header(&headers, "mcp-name")
                        .and_then(decode_name)
                        .as_deref()
                        != params["name"].as_str()))
        {
            return rpc_error(
                id,
                StatusCode::BAD_REQUEST,
                -32020,
                "Method or tool header/body mismatch",
            );
        }
    }
    if method == "initialize" {
        let Some(id) = id else {
            return rpc_error(
                None,
                StatusCode::BAD_REQUEST,
                -32600,
                "Initialization must be a request",
            );
        };
        if modern
            || !params["protocolVersion"].is_string()
            || !params["capabilities"].is_object()
            || !params["clientInfo"]["name"].is_string()
            || !params["clientInfo"]["version"].is_string()
        {
            return rpc_error(
                Some(id),
                StatusCode::BAD_REQUEST,
                -32602,
                "Invalid initialization parameters",
            );
        }
        let requested = params["protocolVersion"].as_str().expect("validated");
        if headers.contains_key("mcp-method")
            && header(&headers, "mcp-method") != Some("initialize")
        {
            return rpc_error(
                Some(id),
                StatusCode::BAD_REQUEST,
                -32020,
                "Initialization method mismatch",
            );
        }
        if headers.contains_key("mcp-protocol-version") && supplied.is_none() {
            return rpc_error(
                Some(id),
                StatusCode::BAD_REQUEST,
                -32020,
                "Invalid protocol version header",
            );
        }
        if supplied.is_some_and(|v| v != requested) {
            return rpc_error(
                Some(id),
                StatusCode::BAD_REQUEST,
                -32020,
                "Initialization version mismatch",
            );
        }
        let version = if LEGACY.contains(&requested) {
            requested
        } else {
            LEGACY[0]
        };
        let mut response = json_response(
            StatusCode::OK,
            result(
                id.clone(),
                json!({"protocolVersion":version,"serverInfo":server_info(),"capabilities":capabilities(),"instructions":INSTRUCTIONS}),
                false,
            ),
        );
        response.headers_mut().insert(
            "mcp-session-id",
            session(&state.key, identity.user_id, version)
                .parse()
                .expect("session header"),
        );
        return response;
    }
    let legacy_session = if modern {
        None
    } else {
        let Some(handle) = header(&headers, "mcp-session-id") else {
            return rpc_error(
                id,
                StatusCode::BAD_REQUEST,
                -32600,
                "Initialize MCP before sending requests",
            );
        };
        let Some(version) = session_version(&state.key, identity.user_id, handle) else {
            return rpc_error(
                id,
                StatusCode::NOT_FOUND,
                -32600,
                "MCP session is unavailable; initialize again",
            );
        };
        if requested.is_some_and(|v| v != version) {
            return rpc_error(
                id,
                StatusCode::BAD_REQUEST,
                -32020,
                "Session protocol version mismatch",
            );
        }
        Some(handle)
    };
    let Some(id) = id.cloned() else {
        if !modern && method == "notifications/cancelled" {
            if !valid_id(&params["requestId"]) {
                return rpc_error(
                    None,
                    StatusCode::BAD_REQUEST,
                    -32602,
                    "Invalid cancellation request",
                );
            }
            let key = request_key(
                &state.key,
                identity.user_id,
                legacy_session.expect("legacy"),
                &params["requestId"],
            );
            if let Some(cancel) = state.mcp.active.lock().expect("MCP requests").get(&key) {
                cancel.cancel();
            }
        }
        return (StatusCode::ACCEPTED, [("cache-control", "no-store")]).into_response();
    };
    match method {
        "server/discover" => json_response(
            StatusCode::OK,
            result(
                id,
                json!({"supportedVersions":[MODERN,"2025-11-25","2025-06-18","2025-03-26"],"capabilities":capabilities(),"instructions":INSTRUCTIONS,"ttlMs":0,"cacheScope":"private"}),
                modern,
            ),
        ),
        "ping" => json_response(StatusCode::OK, result(id, json!({}), modern)),
        "tools/list" => {
            if params.get("cursor").is_some_and(|v| !v.is_null()) {
                return rpc_error(Some(&id), StatusCode::OK, -32602, "Invalid cursor");
            }
            let read_only = params.pointer("/_meta/openai~1readOnly") == Some(&Value::Bool(true));
            let tools = TOOLS
                .iter()
                .filter(|tool| !read_only || tool.read_only())
                .map(|tool| tool.definition())
                .collect::<Vec<_>>();
            let mut catalog = json!({"tools":tools});
            if modern {
                catalog["ttlMs"] = json!(0);
                catalog["cacheScope"] = json!("private");
            }
            json_response(StatusCode::OK, result(id, catalog, modern))
        }
        "tools/call" => {
            if params.as_object().is_some_and(|fields| {
                fields
                    .keys()
                    .any(|key| !["name", "arguments", "_meta"].contains(&key.as_str()))
            }) || params.get("arguments").is_some_and(|v| !v.is_object())
            {
                return rpc_error(
                    Some(&id),
                    StatusCode::OK,
                    -32602,
                    "Unsupported tool call parameters",
                );
            }
            let Some(name) = params["name"].as_str() else {
                return rpc_error(Some(&id), StatusCode::OK, -32602, "Tool name is required");
            };
            let Some(tool) = TOOLS.iter().find(|tool| tool.name() == name).copied() else {
                return rpc_error(Some(&id), StatusCode::OK, -32602, "Unknown tool");
            };
            if params.pointer("/_meta/openai~1readOnly") == Some(&Value::Bool(true))
                && !tool.read_only()
            {
                return tool_error(id, modern, "This tool is not a read-only operation");
            }
            let Some(payload) = tool.payload(&params["arguments"]) else {
                return tool_error(id,modern,"Invalid image arguments: supply a nonempty prompt, supported options and, for editing, one to five inline or HTTPS image URLs");
            };
            call(
                state,
                identity,
                request_id,
                id,
                modern,
                legacy_session,
                tool,
                payload,
            )
            .await
        }
        _ => rpc_error(
            Some(&id),
            if modern {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::OK
            },
            -32601,
            "Method not found",
        ),
    }
}

pub(super) struct ImageReply {
    pub surface: &'static str,
    id: Value,
    modern: bool,
}
pub(super) fn image_result(
    mut image: Value,
    reply: ImageReply,
) -> std::result::Result<Value, &'static str> {
    use base64::{engine::general_purpose::STANDARD, Engine};
    let encoded = image["data"][0]["b64_json"]
        .as_str()
        .ok_or("invalid_response")?;
    // MIME sniffing decodes only a short prefix, never another full image buffer.
    let prefix = &encoded.as_bytes()[..encoded.len().min(32)];
    let mut bytes = [0; 24];
    let length = STANDARD
        .decode_slice(prefix, &mut bytes)
        .map_err(|_| "invalid_response")?;
    let bytes = &bytes[..length];
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        return Err("invalid_response");
    };
    let data = image["data"][0]["b64_json"].take();
    Ok(result(
        reply.id,
        json!({"isError":false,"content":[{"type":"image","data":data,"mimeType":mime},{"type":"text","text":"Image generated using ChatGPT gpt-image-2. Display or save the returned image in the client."}]}),
        reply.modern,
    ))
}

#[allow(clippy::too_many_arguments)]
async fn call(
    state: Arc<AppState>,
    identity: Identity,
    request_id: RequestId,
    id: Value,
    modern: bool,
    session: Option<&str>,
    tool: Tool,
    payload: Value,
) -> Response {
    let permit = match state.mcp.slots.acquire(identity.user_id) {
        Ok(permit) => Arc::new(permit),
        Err(_) => {
            return tool_error(
                id,
                modern,
                "MCP concurrency capacity reached; wait for active requests to finish",
            )
        }
    };
    let cancel = CancellationToken::new();
    let active = if let Some(session) = session {
        let key = request_key(&state.key, identity.user_id, session, &id);
        let mut requests = state.mcp.active.lock().expect("MCP requests");
        if requests.contains_key(&key) {
            return rpc_error(
                Some(&id),
                StatusCode::BAD_REQUEST,
                -32600,
                "Request ID is already in flight",
            );
        }
        if requests.len() >= limits::GLOBAL_LIMIT {
            return tool_error(
                id,
                modern,
                "MCP concurrency capacity reached; wait for active requests to finish",
            );
        }
        requests.insert(key, cancel.clone());
        Some(ActiveRequest {
            state: state.clone(),
            key,
        })
    } else {
        None
    };
    let (mut send, receive) = oneshot::channel();
    let jobs = state.jobs.clone();
    let task_permit = permit.clone();
    jobs.spawn(async move {
        let _task_permit=task_permit;
        let _active=active;
        let response=tokio::select! {
            biased;
            _=send.closed()=>return,
            _=cancel.cancelled()=>return,
            _=wait_for_stop(state.stopped.clone())=>tool_error(id.clone(),modern,"Image request interrupted by shutdown; outcome may be unknown. Do not retry automatically."),
            response=tool.invoke(state.clone(),identity,request_id,id.clone(),modern,payload)=>response,
        };
        let response=if response.status().is_success() {response} else {
            // Native errors are intentionally not parsed or echoed. The actual
            // status/headers already drove quota, refresh and durable accounting.
            let message=match response.status() {
                StatusCode::TOO_MANY_REQUESTS=>"Image capacity or quota is currently unavailable. No automatic retry was attempted.",
                StatusCode::SERVICE_UNAVAILABLE=>"Image service is temporarily unavailable. Check account activation and server status; no automatic retry was attempted.",
                StatusCode::BAD_GATEWAY=>"Image request failed or was interrupted; outcome may be unknown. Do not retry automatically.",
                _=>"Image request was rejected. Check the image request and account entitlement; no automatic retry was attempted.",
            };
            tool_error(id,modern,message)
        };
        let _=send.send(response);
    });
    event_stream(receive, permit)
}

type DataStream =
    Pin<Box<dyn futures_util::Stream<Item = std::result::Result<Bytes, axum::Error>> + Send>>;
enum Phase {
    Waiting(oneshot::Receiver<Response>),
    Sending(DataStream),
    Done,
}
struct StreamState {
    phase: Phase,
    ticker: tokio::time::Interval,
    _permit: Arc<limits::Permit>,
}
fn event_stream(receive: oneshot::Receiver<Response>, permit: Arc<limits::Permit>) -> Response {
    let mut ticker = tokio::time::interval(Duration::from_secs(15));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let stream = futures_util::stream::unfold(
        StreamState {
            phase: Phase::Waiting(receive),
            ticker,
            _permit: permit,
        },
        |mut state| async move {
            let bytes = match &mut state.phase {
                Phase::Waiting(receive) => tokio::select! {
                    response=receive=>{
                        let Ok(response)=response else {return None};
                        state.phase=Phase::Sending(Box::pin(response.into_body().into_data_stream()));
                        Bytes::from_static(b"event: message\ndata: ")
                    }
                    _=state.ticker.tick()=>Bytes::from_static(b": ping\n\n"),
                },
                Phase::Sending(body) => match body.next().await {
                    Some(Ok(bytes)) => bytes,
                    Some(Err(_)) => return None,
                    None => {
                        state.phase = Phase::Done;
                        Bytes::from_static(b"\n\n")
                    }
                },
                Phase::Done => return None,
            };
            Some((Ok::<_, Infallible>(bytes), state))
        },
    );
    (
        StatusCode::OK,
        [
            ("content-type", "text/event-stream"),
            ("cache-control", "no-store"),
            ("x-accel-buffering", "no"),
        ],
        Body::from_stream(stream),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_and_response_body_both_keep_mcp_capacity_reserved() {
        let state = McpState::new(1);
        let body = Arc::new(state.slots.acquire(1).unwrap());
        let worker = body.clone();
        drop(body);
        assert_eq!(state.slots.acquire(1).err(), Some(limits::Full::User));
        drop(worker);
        assert!(state.slots.acquire(1).is_ok());
    }
}
