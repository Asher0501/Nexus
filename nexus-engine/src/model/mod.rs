//! Core data types for workflow definitions, configuration, and errors.
//!
//! This module defines the fundamental types that describe a workflow:
//! - [`WorkflowDef`]: The top-level workflow definition
//! - [`NodeDef`]: Individual node definitions
//! - [`PredecessorDef`]: Declarations of predecessor relationships
//! - [`ProviderDef`]: How a node is executed (subprocess, HTTP, etc.)
//! - [`EngineConfig`]: Runtime engine configuration
//! - [`ValidationError`] and [`BuildError`]: Error types

/// Workflow and node definitions (`WorkflowDef`, `NodeDef`).
pub mod workflow;

/// Provider type definitions (`ProviderDef` enum with Subprocess/Http variants).
pub mod provider;

/// Predecessor relationship types (`PredecessorDef`, `TriggerExpr`, `EventType`).
pub mod predecessor;

/// Engine runtime configuration (`EngineConfig`).
pub mod config;

/// Error types for validation and build phases (`ValidationError`, `BuildError`).
pub mod error;

pub use workflow::{WorkflowDef, NodeDef};
pub use provider::ProviderDef;
pub use predecessor::{
    default_threshold, DataFlowDef, EventType, PredecessorDef, SchedulingEdgeDef, TriggerExpr,
};
pub use config::EngineConfig;
pub use error::{ValidationError, BuildError};

#[cfg(test)]
mod tests {
    use crate::model::*;
    use serde_json;

    fn valid_minimal_json() -> &'static str {
        r#"{"nodes":[{"id":"a","providers":[{"type":"subprocess","command":"echo hi"}],"process_timeout_secs":30}]}"#
    }

    #[test]
    fn test_deserialize_minimal_workflow() {
        let wf: WorkflowDef =
            serde_json::from_str(valid_minimal_json()).expect("valid minimal JSON should parse");
        assert_eq!(wf.nodes.len(), 1);
        assert_eq!(wf.nodes[0].id, "a");
    }

    #[test]
    fn test_deserialize_default_threshold() {
        let json = r#"{"nodes":[{"id":"a","providers":[{"type":"subprocess","command":"echo"}],"process_timeout_secs":10}],"edges":[{"from":"b","to":"a","trigger":"all","event":"complete"}]}"#;
        let wf: WorkflowDef = serde_json::from_str(json).expect("valid JSON should parse");
        assert_eq!(wf.edges[0].threshold, 1);
    }

    #[test]
    fn test_deserialize_default_inputs() {
        let json = r#"{"nodes":[{"id":"a","providers":[{"type":"subprocess","command":"echo"}],"process_timeout_secs":10}],"edges":[]}"#;
        let wf: WorkflowDef = serde_json::from_str(json).expect("valid JSON should parse");
        assert!(wf.dataflows.is_empty());
    }

    #[test]
    fn test_serialize_roundtrip() {
        let wf: WorkflowDef =
            serde_json::from_str(valid_minimal_json()).expect("valid minimal JSON should parse");
        let serialized = serde_json::to_string(&wf).expect("serialization should succeed");
        let deserialized: WorkflowDef =
            serde_json::from_str(&serialized).expect("roundtrip JSON should parse");
        assert_eq!(wf, deserialized);
    }

    #[test]
    fn test_deserialize_fan_in_predecessors() {
        let json = r#"{
            "nodes": [{"id":"fan_in","providers":[{"type":"subprocess","command":"echo"}],"process_timeout_secs":10}],
            "edges": [
                {"from":"A","to":"fan_in","trigger":"all","event":"complete"},
                {"from":"B","to":"fan_in","trigger":"all","event":"complete","threshold":3}
            ]
        }"#;
        let wf: WorkflowDef = serde_json::from_str(json).expect("valid JSON should parse");
        assert_eq!(wf.edges.len(), 2);
        assert_eq!(wf.edges[0].threshold, 1);
        assert_eq!(wf.edges[1].threshold, 3);
    }

    #[test]
    fn test_deserialize_invalid_json() {
        let result: Result<WorkflowDef, _> = serde_json::from_str("{invalid}");
        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_empty_nodes() {
        let json = r#"{"nodes":[]}"#;
        let wf: WorkflowDef = serde_json::from_str(json).expect("empty nodes JSON should parse");
        assert!(wf.nodes.is_empty());
        assert!(wf.edges.is_empty());
        assert!(wf.dataflows.is_empty());
    }

    #[test]
    fn test_engine_config_defaults() {
        let config = EngineConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.default_node_timeout_secs, 3600);
    }

    #[test]
    fn test_engine_config_effective_concurrency() {
        let config = EngineConfig::default();
        let effective = config.effective_max_concurrency();
        assert!(effective >= 1);
    }

    #[test]
    fn test_validation_error_display() {
        let err = ValidationError::EmptyGraph;
        assert!(!err.to_string().is_empty());

        let err = ValidationError::DuplicateNodeId { node_id: "x".into() };
        assert!(err.to_string().contains("x"));
    }

    #[test]
    fn test_build_error_display() {
        let err = BuildError::InvalidNodeIndex { description: "index out of bounds".into() };
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn test_deserialize_with_returns() {
        let json = r#"{"nodes":[{"id":"a","providers":[{"type":"subprocess","command":"echo"}],"process_timeout_secs":10,"returns":["approved","rejected"]}],"edges":[]}"#;
        let wf: WorkflowDef = serde_json::from_str(json).expect("returns JSON should parse");
        assert_eq!(wf.nodes[0].returns, vec!["approved", "rejected"]);
    }

    #[test]
    fn test_deserialize_provider_def_subprocess() {
        let json = r#"{"type":"subprocess","command":"python script.py"}"#;
        let provider: ProviderDef =
            serde_json::from_str(json).expect("valid provider JSON should parse");
        assert_eq!(
            provider,
            ProviderDef::Subprocess { command: "python script.py".into() }
        );
    }

    #[test]
    fn test_trigger_expr_serialize() {
        assert_eq!(
            serde_json::to_string(&TriggerExpr::All).expect("serialize All"),
            r#""all""#
        );
        assert_eq!(
            serde_json::to_string(&TriggerExpr::Any).expect("serialize Any"),
            r#""any""#
        );
    }

    #[test]
    fn test_event_type_serialize() {
        assert_eq!(
            serde_json::to_string(&EventType::Complete).expect("serialize Complete"),
            r#""complete""#
        );
        assert_eq!(
            serde_json::to_string(&EventType::Failed).expect("serialize Failed"),
            r#""failed""#
        );
        assert_eq!(
            serde_json::to_string(&EventType::Timeout).expect("serialize Timeout"),
            r#""timeout""#
        );
    }

    // ── 端到端集成测试 ──────────────────────────────────
    //
    // 使用 ARCHITECTURE.md §2 中的真实示例工作流 JSON。
    // 测试完整路径：JSON → deserialize → serialize → re-deserialize → compare

    /// ARCHITECTURE.md §2 的完整示例工作流（含 edges、dataflows、returns）
    fn architecture_example_json() -> &'static str {
        r#"{
            "nodes": [
                {
                    "id": "fetch_data",
                    "providers": [{ "type": "subprocess", "command": "python plugins/fetcher.py" }],
                    "process_timeout_secs": 30,
                    "max_concurrency": 1
                },
                {
                    "id": "validate",
                    "providers": [{ "type": "subprocess", "command": "python plugins/validator.py" }],
                    "process_timeout_secs": 10
                },
                {
                    "id": "review",
                    "providers": [{ "type": "subprocess", "command": "python plugins/reviewer.py" }],
                    "process_timeout_secs": 120,
                    "returns": ["approved", "rejected"]
                }
            ],
            "edges": [
                { "from": "fetch_data", "to": "validate", "trigger": "all", "event": "complete" },
                { "from": "generate_code", "to": "review", "trigger": "all", "event": "complete", "threshold": 5 }
            ],
            "dataflows": [
                { "from": "fetch_data", "to": "validate" },
                { "from": "generate_code", "to": "review" }
            ]
        }"#
    }

    #[test]
    fn test_e2e_architecture_example_parse() {
        // 端到端 1：解析 ARCHITECTURE.md 的示例 JSON → 验证结构
        let wf: WorkflowDef = serde_json::from_str(architecture_example_json())
            .expect("ARCHITECTURE.md example JSON should parse");
        assert_eq!(wf.nodes.len(), 3, "example has 3 nodes");

        // 验证节点 1：fetch_data（入口节点）
        let fetch = &wf.nodes[0];
        assert_eq!(fetch.id, "fetch_data");
        assert_eq!(fetch.max_concurrency, Some(1));
        assert!(fetch.returns.is_empty(), "entry node has no returns");

        // 验证边 1：fetch_data → validate（All/Complete，threshold 默认 = 1）
        assert_eq!(wf.edges.len(), 2);
        let edge_validate = &wf.edges[0];
        assert_eq!(edge_validate.from, "fetch_data");
        assert_eq!(edge_validate.to, "validate");
        assert_eq!(edge_validate.trigger, TriggerExpr::All);
        assert_eq!(edge_validate.event, EventType::Complete);
        assert_eq!(edge_validate.threshold, 1, "default threshold must be 1");

        // 验证 dataflows
        assert_eq!(wf.dataflows.len(), 2);
        assert_eq!(wf.dataflows[0].from, "fetch_data");
        assert_eq!(wf.dataflows[0].to, "validate");

        // 验证节点 3：review（threshold=5, returns）
        let review = &wf.nodes[2];
        assert_eq!(review.id, "review");
        assert_eq!(review.returns, vec!["approved", "rejected"]);
        let edge_review = &wf.edges[1];
        assert_eq!(edge_review.from, "generate_code");
        assert_eq!(edge_review.to, "review");
        assert_eq!(edge_review.threshold, 5);
        assert_eq!(wf.dataflows[1].from, "generate_code");
        assert_eq!(wf.dataflows[1].to, "review");
    }

    #[test]
    fn test_e2e_architecture_example_roundtrip() {
        // 端到端 2：序列化后重新反序列化，验证不变性
        let original: WorkflowDef = serde_json::from_str(architecture_example_json())
            .expect("original parse");
        let serialized = serde_json::to_string_pretty(&original)
            .expect("serialize");
        let recovered: WorkflowDef = serde_json::from_str(&serialized)
            .expect("roundtrip parse");

        assert_eq!(original, recovered, "ARCHITECTURE.md example must survive roundtrip");
    }

    #[test]
    fn test_e2e_node_with_all_fields() {
        // 端到端 3：所有可选字段都填满的节点
        let json = r#"{
            "nodes": [{
                "id": "full_node",
                "providers": [
                    {"type": "subprocess", "command": "step1"},
                    {"type": "http", "url": "http://api.example.com", "method": "POST"}
                ],
                "process_timeout_secs": 60,
                "max_concurrency": 4,
                "returns": ["pass", "fail"],
                "max_retries": 5
            }],
            "edges": [
                {"from": "A", "to": "full_node", "trigger": "all", "event": "complete", "exit_reason": "ok", "threshold": 2},
                {"from": "B", "to": "full_node", "trigger": "any", "event": "failed"}
            ],
            "dataflows": [
                {"from": "A", "to": "full_node"},
                {"from": "B", "to": "full_node"},
                {"from": "C", "to": "full_node"}
            ]
        }"#;
        let wf: WorkflowDef = serde_json::from_str(json)
            .expect("full fields JSON should parse");
        let node = &wf.nodes[0];

        assert_eq!(node.id, "full_node");
        assert_eq!(node.providers.len(), 2);
        assert_eq!(node.max_concurrency, Some(4));
        assert_eq!(node.returns, vec!["pass", "fail"]);
        assert_eq!(node.max_retries, Some(5));

        // 验证 edges
        assert_eq!(wf.edges.len(), 2);
        assert_eq!(wf.edges[0].from, "A");
        assert_eq!(wf.edges[0].to, "full_node");
        assert_eq!(wf.edges[0].trigger, TriggerExpr::All);
        assert_eq!(wf.edges[0].event, EventType::Complete);
        assert_eq!(wf.edges[0].exit_reason.as_deref(), Some("ok"));
        assert_eq!(wf.edges[0].threshold, 2);
        assert_eq!(wf.edges[1].from, "B");
        assert_eq!(wf.edges[1].trigger, TriggerExpr::Any);
        assert_eq!(wf.edges[1].event, EventType::Failed);

        // 验证 dataflows
        assert_eq!(wf.dataflows.len(), 3);

        // 验证 HTTP provider
        match &node.providers[1] {
            ProviderDef::Http { url, method } => {
                assert_eq!(url, "http://api.example.com");
                assert_eq!(method.as_deref(), Some("POST"));
            }
            _ => panic!("second provider should be Http"),
        }

        // 验证 roundtrip
        let serialized = serde_json::to_string(&wf).expect("serialize");
        let recovered: WorkflowDef = serde_json::from_str(&serialized)
            .expect("roundtrip");
        assert_eq!(wf, recovered);
    }

    #[test]
    fn test_e2e_missing_all_optional_fields() {
        // 端到端 4：没有任何可选字段的最小节点
        let json = r#"{"nodes":[{"id":"min","providers":[{"type":"subprocess","command":"x"}],"process_timeout_secs":5}],"edges":[]}"#;
        let wf: WorkflowDef = serde_json::from_str(json)
            .expect("minimal JSON should parse");
        let node = &wf.nodes[0];

        assert!(wf.edges.is_empty(), "edges defaults to empty");
        assert!(wf.dataflows.is_empty(), "dataflows defaults to empty");
        assert!(node.returns.is_empty(), "returns defaults to empty");
        assert_eq!(node.max_concurrency, None);
        assert_eq!(node.max_retries, None);
    }

    #[test]
    fn test_e2e_engine_config_json_roundtrip() {
        // 端到端 5：EngineConfig 的序列化/反序列化
        let config = EngineConfig::new(Some(8), 7200, 5);
        let json = serde_json::to_string(&config).expect("serialize config");
        let recovered: EngineConfig = serde_json::from_str(&json)
            .expect("deserialize config");
        assert_eq!(config, recovered, "EngineConfig roundtrip");

        // 默认配置序列化后可以重新读回
        let default_config = EngineConfig::default();
        let json2 = serde_json::to_string(&default_config).expect("serialize default config");
        let recovered2: EngineConfig = serde_json::from_str(&json2)
            .expect("deserialize default config");
        assert_eq!(default_config, recovered2);
    }

    #[test]
    fn test_e2e_provider_all_variants() {
        // 端到端 6：所有 ProviderDef 变体的解析
        // Subprocess
        let sp: ProviderDef = serde_json::from_str(r#"{"type":"subprocess","command":"ls"}"#)
            .expect("subprocess");
        assert_eq!(sp, ProviderDef::Subprocess { command: "ls".into() });

        // Http with method
        let http: ProviderDef = serde_json::from_str(
            r#"{"type":"http","url":"https://example.com/api","method":"GET"}"#,
        ).expect("http with method");
        assert_eq!(
            http,
            ProviderDef::Http {
                url: "https://example.com/api".into(),
                method: Some("GET".into()),
            }
        );

        // Http without method（默认 None）
        let http2: ProviderDef = serde_json::from_str(
            r#"{"type":"http","url":"https://example.com/api"}"#,
        ).expect("http without method");
        assert_eq!(
            http2,
            ProviderDef::Http {
                url: "https://example.com/api".into(),
                method: None,
            }
        );
    }

    #[test]
    fn test_e2e_validation_error_all_variants_display() {
        // 端到端 7：所有 ValidationError 变体的 Display 输出不为空
        let cases: Vec<ValidationError> = vec![
            ValidationError::NoEntryNode,
            ValidationError::UnreachableNode { node_id: "a".into() },
            ValidationError::ExitNotReachable { node_id: "b".into() },
            ValidationError::CycleWithoutEntry,
            ValidationError::EmptyGraph,
            ValidationError::NoValidProvider { node_id: "c".into() },
            ValidationError::DuplicateNodeId { node_id: "d".into() },
            ValidationError::InvalidPredecessor { node_id: "e".into(), predecessor_id: "f".into() },
            ValidationError::InputSourceNotFound { node_id: "g".into(), source_id: "h".into() },
            ValidationError::InputSourceUnreachable { node_id: "i".into(), source_id: "j".into() },
        ];

        for (i, err) in cases.iter().enumerate() {
            let msg = err.to_string();
            assert!(!msg.is_empty(), "ValidationError variant {} display should not be empty", i);
            // Verify Error trait is implemented
            let _: &dyn std::error::Error = err;
        }
    }

    #[test]
    fn test_e2e_predecessor_def_with_all_fields() {
        // 端到端 8：PredecessorDef 多字段组合
        let json = r#"{
            "node_id": "upstream",
            "trigger": "any",
            "event": "failed",
            "exit_reason": "crash",
            "threshold": 3
        }"#;
        let pred: PredecessorDef = serde_json::from_str(json)
            .expect("predecessor with all fields");
        assert_eq!(pred.node_id, "upstream");
        assert_eq!(pred.trigger, TriggerExpr::Any);
        assert_eq!(pred.event, EventType::Failed);
        assert_eq!(pred.exit_reason.as_deref(), Some("crash"));
        assert_eq!(pred.threshold, 3);
    }
}
