pub mod ast;
pub mod dag;
pub mod parser;
pub mod protocol;

pub use ast::*;
pub use dag::{DagError, JobDag};
pub use parser::{ParseError, WorkflowParser};
pub use protocol::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_yaml_parsing_and_dag() {
        let yaml = r#"
name: "Continuous Integration"
on:
  push:
    branches: [main, "release/*"]
  pull_request:

env:
  RUST_BACKTRACE: "1"

jobs:
  lint:
    runs-on: [self-hosted, windows]
    steps:
      - name: "Checkout source"
        uses: "actions/checkout@v1"
      - name: "Run Clippy"
        run: "cargo clippy --all-targets"

  test:
    runs-on: [self-hosted, windows]
    needs: [lint]
    steps:
      - name: "Run tests"
        run: |
          cargo test --workspace
          echo "All tests passed"

  build:
    runs-on: [self-hosted, windows]
    needs: [test]
    steps:
      - name: "Build Release Binary"
        run: "cargo build --release"
"#;

        let spec = WorkflowParser::parse(yaml).expect("YAML parse should succeed");
        assert_eq!(spec.name, "Continuous Integration");
        assert!(spec.on.push.is_some());
        assert!(spec.on.pull_request.is_some());
        assert_eq!(spec.on.push.as_ref().unwrap().branches, vec!["main", "release/*"]);
        assert_eq!(spec.jobs.len(), 3);

        let dag = JobDag::build(&spec).expect("DAG build should succeed");
        let order = dag.topological_order();
        assert_eq!(order, vec!["lint", "test", "build"]);

        let mut completed = std::collections::HashSet::new();
        let mut in_progress = std::collections::HashSet::new();

        // Initially only 'lint' is ready
        let ready = dag.ready_jobs(&completed, &in_progress);
        assert_eq!(ready, vec!["lint"]);

        // Complete 'lint'
        completed.insert("lint".to_string());
        in_progress.insert("lint".to_string());
        let ready = dag.ready_jobs(&completed, &in_progress);
        assert_eq!(ready, vec!["test"]);

        // Complete 'test'
        completed.insert("test".to_string());
        in_progress.insert("test".to_string());
        let ready = dag.ready_jobs(&completed, &in_progress);
        assert_eq!(ready, vec!["build"]);
    }

    #[test]
    fn test_workflow_cycle_detection() {
        let yaml = r#"
name: "Cyclic Pipeline"
on: [push]
jobs:
  job_a:
    needs: [job_c]
    steps:
      - run: "echo A"
  job_b:
    needs: [job_a]
    steps:
      - run: "echo B"
  job_c:
    needs: [job_b]
    steps:
      - run: "echo C"
"#;

        let spec = WorkflowParser::parse(yaml).expect("YAML parse should succeed");
        let err = JobDag::build(&spec).expect_err("Cyclic DAG must fail");
        match err {
            DagError::CycleDetected(cycle) => {
                assert!(cycle.contains("job_a"));
                assert!(cycle.contains("job_b"));
                assert!(cycle.contains("job_c"));
            }
            _ => panic!("Expected CycleDetected error"),
        }
    }

    #[test]
    fn test_workflow_validation_rejects_empty_and_missing_steps() {
        let invalid_yaml = r#"
name: ""
on: push
jobs:
  job_empty:
    steps: []
"#;
        assert!(WorkflowParser::parse(invalid_yaml).is_err());
    }
}
