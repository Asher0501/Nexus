//! Shared template engine for rendering `{{metadata.*}}` and
//! `{{datarouter.*.*}}` placeholders in command and prompt templates.
//!
//! Used by both [`SubprocessExecutor`](super::SubprocessExecutor) and
//! [`LlmExecutor`](super::LlmExecutor) to avoid duplicate parsing logic.

use std::collections::HashMap;

use super::types::{NodeMetadata, NodeOutput};

/// Renders `{{metadata.*}}` and `{{datarouter.*.*}}` placeholders.
///
/// # Supported patterns
///
/// | Pattern | Example | Description |
/// |---|---|---|
/// | `{{metadata.run_count}}` | → `3` | Current execution count (1-based) |
/// | `{{metadata.timed_out}}` | → `true` | Whether previous run timed out |
/// | `{{datarouter.<alias>.route}}` | → `approved` | Upstream node's route value |
/// | `{{datarouter.<alias>.content}}` | → `...` | Upstream node's content |
/// | `{{node_dir}}` | → `/path/to/scripts` | Resolved scripts directory |
///
/// Unknown metadata fields and unrecognised prefixes produce a `tracing::warn`
/// and pass the original `{{...}}` through unchanged so the workflow author can
/// diagnose the issue without losing context.
pub struct TemplateEngine;

impl TemplateEngine {
    /// Render placeholders, substituting values directly (no escaping).
    ///
    /// This is the right choice for prompt templates that will be passed as
    /// structured data (e.g. inside a JSON context to `llm_node.py`).
    #[must_use]
    pub fn render(
        template: &str,
        metadata: &NodeMetadata,
        upstream: &HashMap<String, NodeOutput>,
        scripts_dir: &str,
    ) -> String {
        Self::render_impl(template, metadata, upstream, scripts_dir, false)
    }

    /// Render placeholders with shell-safe escaping.
    #[must_use]
    pub fn render_shell(
        template: &str,
        metadata: &NodeMetadata,
        upstream: &HashMap<String, NodeOutput>,
        scripts_dir: &str,
    ) -> String {
        Self::render_impl(template, metadata, upstream, scripts_dir, true)
    }

    fn render_impl(
        template: &str,
        metadata: &NodeMetadata,
        upstream: &HashMap<String, NodeOutput>,
        scripts_dir: &str,
        shell: bool,
    ) -> String {
        let mut result = String::with_capacity(template.len());
        let mut rest = template;

        while let Some(start) = rest.find("{{") {
            result.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find("}}") else {
                // Unclosed placeholder — pass through unchanged.
                result.push_str(&rest[start..]);
                rest = "";
                break;
            };
            let key_path = &after[..end];
            let consumed = start + 2 + end + 2;

            if let Some(meta_key) = key_path.strip_prefix("metadata.") {
                let value = match meta_key {
                    "run_count" => metadata.run_count.to_string(),
                    "timed_out" => metadata.timed_out.to_string(),
                    field => {
                        tracing::warn!(
                            "unknown metadata field '{}' — valid: run_count, timed_out",
                            field,
                        );
                        // Pass through unchanged so the author can see what went wrong.
                        result.push_str(&rest[start..start + 4 + end]);
                        rest = &rest[consumed..];
                        continue;
                    }
                };
                if shell {
                    result.push_str(&Self::shell_escape(&value));
                } else {
                    result.push_str(&value);
                }
            } else if let Some(dr_path) = key_path.strip_prefix("datarouter.") {
                if let Some(dot) = dr_path.find('.') {
                    let alias = &dr_path[..dot];
                    let field = &dr_path[dot + 1..];
                    let fallback = NodeOutput {
                        route: String::new(),
                        content: String::new(),
                    };
                    let output = upstream.get(alias).unwrap_or(&fallback);
                    let value = match field {
                        "route" => output.route.clone(),
                        "content" => output.content.clone(),
                        field => {
                            tracing::warn!(
                                "unknown datarouter field '{}' for source '{}' — valid: route, content",
                                field,
                                alias,
                            );
                            result.push_str(&rest[start..start + 4 + end]);
                            rest = &rest[consumed..];
                            continue;
                        }
                    };
                    if shell {
                        result.push_str(&Self::shell_escape(&value));
                    } else {
                        result.push_str(&value);
                    }
                } else {
                    // Malformed datarouter path (no dot after alias) — pass through.
                    result.push_str(&rest[start..start + 4 + end]);
                }
            } else if key_path == "node_dir" {
                if shell {
                    result.push_str(&Self::shell_escape(scripts_dir));
                } else {
                    result.push_str(scripts_dir);
                }
            } else if key_path.contains('.') {
                tracing::warn!(
                    "unrecognized template '{}' — engine only supports metadata.*, datarouter.*.*, and node_dir",
                    key_path,
                );
                result.push_str(&rest[start..start + 4 + end]);
            } else {
                // Plain {{x}} without dot — pass through unchanged (likely LLM prompt example).
                result.push_str(&rest[start..start + 4 + end]);
            }
            rest = &rest[consumed..];
        }

        result.push_str(rest);
        result
    }

    /// Escape a value for safe interpolation into a shell command.
    ///
    /// - **Windows cmd.exe**: wraps in double quotes, escapes internal `"` →
    ///   `""` and `%` → `%%`.
    /// - **Unix sh**: wraps in single quotes, escapes internal `'` → `'\''`.
    fn shell_escape(value: &str) -> String {
        if cfg!(windows) {
            let escaped = value.replace('"', "\"\"").replace('%', "%%");
            format!("\"{escaped}\"")
        } else {
            let escaped = value.replace('\'', "'\\''");
            format!("'{escaped}'")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(run_count: u64, timed_out: bool) -> NodeMetadata {
        NodeMetadata { run_count, timed_out }
    }

    fn upstream_with(alias: &str, route: &str, content: &str) -> HashMap<String, NodeOutput> {
        let mut m = HashMap::new();
        m.insert(alias.into(), NodeOutput { route: route.into(), content: content.into() });
        m
    }

    const SD: &str = "/my/scripts";

    // ── plain render ──────────────────────────────────────────

    #[test]
    fn plain_no_template() {
        assert_eq!(TemplateEngine::render("echo hello", &meta(1, false), &HashMap::new(), SD), "echo hello");
    }

    #[test]
    fn plain_metadata_run_count() {
        assert_eq!(TemplateEngine::render("round_{{metadata.run_count}}", &meta(5, false), &HashMap::new(), SD), "round_5");
    }

    #[test]
    fn plain_datarouter_route() {
        assert_eq!(TemplateEngine::render("{{datarouter.up.route}}", &meta(1, false), &upstream_with("up", "complete", "data"), SD), "complete");
    }

    #[test]
    fn plain_datarouter_content() {
        assert_eq!(TemplateEngine::render("{{datarouter.up.content}}", &meta(1, false), &upstream_with("up", "ok", "hello world"), SD), "hello world");
    }

    #[test]
    fn plain_node_dir() {
        assert_eq!(TemplateEngine::render("{{node_dir}}/tool.py", &meta(1, false), &HashMap::new(), SD), "/my/scripts/tool.py");
    }

    // ── shell render ──────────────────────────────────────────

    #[test]
    fn shell_escapes_spaces() {
        let result = TemplateEngine::render_shell("{{datarouter.up.content}}", &meta(1, false), &upstream_with("up", "ok", "hello world"), SD);
        assert!(result.contains("hello world"));
        assert!(result.contains('\'') || result.contains('"'));
    }

    #[test]
    fn shell_node_dir() {
        let result = TemplateEngine::render_shell("python {{node_dir}}/tool.py", &meta(1, false), &HashMap::new(), "/path/with spaces");
        assert!(result.contains("/path/with spaces"));
        assert!(result.contains('\'') || result.contains('"'));
    }

    // ── edge cases ────────────────────────────────────────────

    #[test]
    fn unknown_metadata_field_passthrough() {
        assert_eq!(TemplateEngine::render("{{metadata.unknown}}", &meta(1, false), &HashMap::new(), SD), "{{metadata.unknown}}");
    }

    #[test]
    fn missing_upstream_uses_fallback() {
        assert_eq!(TemplateEngine::render("{{datarouter.missing.route}}", &meta(1, false), &HashMap::new(), SD), "");
    }

    #[test]
    fn plain_braces_without_dot_passthrough() {
        assert_eq!(TemplateEngine::render("{{plain}}", &meta(1, false), &HashMap::new(), SD), "{{plain}}");
    }

    #[test]
    fn unclosed_placeholder_passthrough() {
        assert_eq!(TemplateEngine::render("hello {{world", &meta(1, false), &HashMap::new(), SD), "hello {{world");
    }
}
