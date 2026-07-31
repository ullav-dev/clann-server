pub mod server;

/// Builds a `CallToolResult` carrying both a tool's human-readable text (for
/// chat/LLM callers, unchanged from before) and the same underlying data as
/// `structured_content` — MCP's dedicated machine-readable result field. Lets
/// an awe automated task bind a named field directly instead of regex-scraping
/// the prose. Matches the same helper in awe-server's `mcp::text_result`, kept
/// consistent across every first-party MCP server.
pub(super) fn text_result(text: impl Into<String>, structured: serde_json::Value) -> rmcp::model::CallToolResult {
    let mut result = rmcp::model::CallToolResult::success(vec![rmcp::model::Content::text(text.into())]);
    result.structured_content = Some(structured);
    result
}
