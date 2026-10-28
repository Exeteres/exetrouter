//! Output and transport buffering bounds, independent of model context windows.
//! Inference input has no router-defined byte ceiling. Larger inputs must not
//! implicitly enlarge upstream event or output projection buffers.

pub(crate) const UPSTREAM_WS_FRAME_BYTES: usize = 1024 * 1024;
pub(crate) const UPSTREAM_WS_MESSAGE_BYTES: usize = 1024 * 1024;
pub(crate) const UPSTREAM_SSE_EVENT_BYTES: usize = 1024 * 1024;
pub(crate) const PROJECTED_OUTPUT_BYTES: usize = 1024 * 1024;
pub(crate) const SSE_QUEUE_CHUNKS: usize = 8;
