//! Transport seam for VRChat Chatbox output.
//!
//! Publishers own queueing, pacing, lifecycle, and diagnostics. Transport
//! adapters only attempt already-prepared text and typing packets.

use super::PreparedChatboxText;
use crate::error::AppResult;

pub(crate) trait ChatboxTransport: Send + Sync {
    fn send_text(&self, text: &PreparedChatboxText) -> AppResult<ChatboxSendReceipt>;
    fn send_typing(&self, is_typing: bool) -> AppResult<()>;
}

#[derive(Debug)]
pub(crate) struct ChatboxSendReceipt {
    pub(crate) target: String,
    pub(crate) byte_count: usize,
}
