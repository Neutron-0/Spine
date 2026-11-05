use crate::ast::*;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum ParseError {
    #[error("Parse error at line {line}: {message}")]
    Syntax { line: usize, message: String },
    #[error("Validation error: field '{field}' {message}")]
    Validation { field: String, message: String },
    #[error("JSON error: {0}")]
    Json(String),
}

pub struct WorkflowParser;

impl WorkflowParser {
    /// Parses a YAML or JSON workflow string into a validated `WorkflowSpec`.
    pub fn parse(input: &str) -> Result<WorkflowSpec, ParseError> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err(ParseError::Validation {
                field: "workflow".into(),
                message: "cannot be empty".into(),
            });
        }

        // Try JSON parse first
        if trimmed.starts_with('{') {
            let spec: WorkflowSpec =
                serde_json::from_str(trimmed).map_err(|e| ParseError::Json(e.to_string()))?;
            Self::validate(&spec)?;
            return Ok(spec);
        }

        // Native YAML line parser
        let spec = Self::parse_yaml(input)?;
        Self::validate(&spec)?;
        Ok(spec)
    }

    fn validate(spec: &WorkflowSpec) -> Result<(), ParseError> {
        if spec.name.trim().is_empty() {
            return Err(ParseError::Validation {
                field: "name".into(),
                message: "is required and cannot be empty".into(),
            });
        }

        if spec.jobs.is_empty() {
            return Err(ParseError::Validation {
                field: "jobs".into(),
                message: "must define at least one job".into(),
            });
        }

        for (job_id, job) in &spec.jobs {
            if job_id.trim().is_empty() {
                return Err(ParseError::Validation {
                    field: "job_id".into(),
                    message: "cannot be empty".into(),
                });
            }

            if job.steps.is_empty() {
                return Err(ParseError::Validation {
                    field: format!("jobs.{}.steps", job_id),
                    message: "job must define at least one step".into(),
                });
            }

            for (idx, step) in job.steps.iter().enumerate() {
                if step.run.is_none() && step.uses.is_none() {
                    return Err(ParseError::Validation {
                        field: format!("jobs.{}.steps[{}]", job_id, idx),
                        message: "step must declare either 'run' or 'uses'".into(),
                    });
                }
            }
        }

        Ok(())
    }

    fn parse_yaml(input: &str) -> Result<WorkflowSpec, ParseError> {
        let mut name = String::new();
        let mut on = TriggerSpec::default();
        let mut env = HashMap::new();
        let mut jobs = HashMap::new();

        let mut lines = input.lines().enumerate().peekable();
        let mut current_section = String::new();
        let mut current_event = String::new();

        while let Some((line_no, line)) = lines.next() {
            let _line_num = line_no + 1;
            let trimmed = line.trim();

            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let indent = line.len() - line.trim_start().len();

            if indent == 0 {
                if let Some((key, val)) = Self::split_kv(trimmed) {
                    current_section = key.to_string();
                    current_event.clear();
                    match key {
                        "name" => name = Self::strip_quotes(val),
                        "on" => {
                            if !val.is_empty() {
                                Self::parse_inline_triggers(val, &mut on);
                            }
                        }
                        "env" => {}
                        "jobs" => {}
                        _ => {}
                    }
                } else if trimmed.ends_with(':') {
                    current_section = trimmed.trim_end_matches(':').trim().to_string();
                    current_event.clear();
                }
                continue;
            }

            // Indented sections
            match current_section.as_str() {
                "on" => {
                    if indent == 2 {
                        if let Some((event, filter_val)) = Self::split_kv(trimmed) {
                            current_event = event.to_string();
                            let filter = if filter_val.is_empty() {
                                EventFilter::default()
                            } else {
                                EventFilter {
                                    branches: Self::parse_bracket_list(filter_val),
                                    paths: Vec::new(),
                                }
                            };

                            match event {
                                "push" => on.push = Some(filter),
                                "pull_request" => on.pull_request = Some(filter),
                                "workflow_dispatch" => on.workflow_dispatch = Some(true),
                                _ => {}
                            }
                        } else if trimmed.starts_with('-') {
                            let ev = trimmed.trim_start_matches('-').trim();
                            match ev {
                                "push" => on.push = Some(EventFilter::default()),
                                "pull_request" => on.pull_request = Some(EventFilter::default()),
                                "workflow_dispatch" => on.workflow_dispatch = Some(true),
                                _ => {}
                            }
                        }
                    } else if indent >= 4 {
                        if let Some((k, v)) = Self::split_kv(trimmed) {
                            let list = Self::parse_bracket_list(v);
                            let target_filter = match current_event.as_str() {
                                "push" => on.push.get_or_insert_with(EventFilter::default),
                                "pull_request" => {
                                    on.pull_request.get_or_insert_with(EventFilter::default)
                                }
                                _ => continue,
                            };
                            match k {
                                "branches" => target_filter.branches = list,
                                "paths" => target_filter.paths = list,
                                _ => {}
                            }
                        }
                    }
                }
                "env" => {
                    if let Some((k, v)) = Self::split_kv(trimmed) {
                        env.insert(k.to_string(), Self::strip_quotes(v));
                    }
                }
                "jobs" if indent == 2 && trimmed.ends_with(':') => {
                    // Start of a job definition: indent 2: "build:"
                    let job_id = trimmed.trim_end_matches(':').trim().to_string();
                    let job_spec = Self::parse_job_block(&mut lines)?;
                    jobs.insert(job_id, job_spec);
                }
                _ => {}
            }
        }

        Ok(WorkflowSpec {
            name,
            on,
            env,
            jobs,
        })
    }

    fn parse_job_block<'a, I>(lines: &mut std::iter::Peekable<I>) -> Result<JobSpec, ParseError>
    where
        I: Iterator<Item = (usize, &'a str)>,
    {
        let mut job = JobSpec::default();
        let mut current_step: Option<StepSpec> = None;
        let mut in_multiline_run = false;
        let mut multiline_buf = String::new();
        let mut multiline_indent = 0;

        while let Some(&(line_no, line)) = lines.peek() {
            let _line_num = line_no + 1;
            let trimmed = line.trim();

            if trimmed.is_empty() || trimmed.starts_with('#') {
                lines.next();
                continue;
            }

            let indent = line.len() - line.trim_start().len();

            // End of this job when indent drops to 2 or less
            if indent <= 2 {
                break;
            }

            lines.next();

            // Check if we are capturing a multi-line `run: |`
            if in_multiline_run {
                if indent >= multiline_indent {
                    if !multiline_buf.is_empty() {
                        multiline_buf.push('\n');
                    }
                    multiline_buf.push_str(trimmed);
                    continue;
                } else {
                    // Exited multiline run
                    if let Some(step) = &mut current_step {
                        step.run = Some(multiline_buf.clone());
                    }
                    in_multiline_run = false;
                    multiline_buf.clear();
                }
            }

            if indent == 4 {
                // Job level fields: name, runs-on, needs, steps
                if let Some(step) = current_step.take() {
                    job.steps.push(step);
                }

                if let Some((key, val)) = Self::split_kv(trimmed) {
                    match key {
                        "name" => job.name = Some(Self::strip_quotes(val)),
                        "runs-on" => {
                            if val.starts_with('[') {
                                job.runs_on = Self::parse_bracket_list(val);
                            } else if !val.is_empty() {
                                job.runs_on = vec![Self::strip_quotes(val)];
                            }
                        }
                        "needs" => {
                            if val.starts_with('[') {
                                job.needs = Self::parse_bracket_list(val);
                            } else if !val.is_empty() {
                                job.needs = vec![Self::strip_quotes(val)];
                            }
                        }
                        "timeout-minutes" | "timeout_minutes" => {
                            job.timeout_minutes = val.parse().ok();
                        }
                        "continue-on-error" | "continue_on_error" => {
                            job.continue_on_error = Some(val == "true");
                        }
                        _ => {}
                    }
                }
                continue;
            }

            // Steps parsing
            if indent == 6 && trimmed.starts_with('-') {
                if let Some(step) = current_step.take() {
                    job.steps.push(step);
                }

                let mut step = StepSpec::default();
                let remainder = trimmed.trim_start_matches('-').trim();
                if let Some((k, v)) = Self::split_kv(remainder) {
                    Self::apply_step_kv(
                        &mut step,
                        k,
                        v,
                        &mut in_multiline_run,
                        &mut multiline_buf,
                        &mut multiline_indent,
                        indent + 2,
                    );
                }
                current_step = Some(step);
                continue;
            }

            if indent >= 8 {
                if let Some(step) = &mut current_step {
                    if let Some((k, v)) = Self::split_kv(trimmed) {
                        Self::apply_step_kv(
                            step,
                            k,
                            v,
                            &mut in_multiline_run,
                            &mut multiline_buf,
                            &mut multiline_indent,
                            indent + 2,
                        );
                    }
                }
            }
        }

        if in_multiline_run {
            if let Some(step) = &mut current_step {
                step.run = Some(multiline_buf);
            }
        }

        if let Some(step) = current_step {
            job.steps.push(step);
        }

        Ok(job)
    }

    fn apply_step_kv(
        step: &mut StepSpec,
        k: &str,
        v: &str,
        in_multiline: &mut bool,
        multiline_buf: &mut String,
        multiline_indent: &mut usize,
        expected_indent: usize,
    ) {
        match k {
            "id" => step.id = Some(Self::strip_quotes(v)),
            "name" => step.name = Some(Self::strip_quotes(v)),
            "uses" => step.uses = Some(Self::strip_quotes(v)),
            "run" => {
                if v == "|" || v == ">" {
                    *in_multiline = true;
                    multiline_buf.clear();
                    *multiline_indent = expected_indent;
                } else {
                    step.run = Some(Self::strip_quotes(v));
                }
            }
            "working-directory" | "working_directory" => {
                step.working_directory = Some(Self::strip_quotes(v));
            }
            _ => {}
        }
    }

    fn split_kv(s: &str) -> Option<(&str, &str)> {
        let idx = s.find(':')?;
        let key = s[..idx].trim();
        let val = s[idx + 1..].trim();
        Some((key, val))
    }

    fn strip_quotes(s: &str) -> String {
        let trimmed = s.trim();
        if ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
            && trimmed.len() >= 2
        {
            return trimmed[1..trimmed.len() - 1].to_string();
        }
        trimmed.to_string()
    }

    fn parse_bracket_list(s: &str) -> Vec<String> {
        let trimmed = s.trim().trim_start_matches('[').trim_end_matches(']');
        trimmed
            .split(',')
            .map(|item| Self::strip_quotes(item.trim()))
            .filter(|item| !item.is_empty())
            .collect()
    }

    fn parse_inline_triggers(val: &str, on: &mut TriggerSpec) {
        if val.starts_with('[') {
            for item in Self::parse_bracket_list(val) {
                match item.as_str() {
                    "push" => on.push = Some(EventFilter::default()),
                    "pull_request" => on.pull_request = Some(EventFilter::default()),
                    "workflow_dispatch" => on.workflow_dispatch = Some(true),
                    _ => {}
                }
            }
        } else {
            let single = Self::strip_quotes(val);
            match single.as_str() {
                "push" => on.push = Some(EventFilter::default()),
                "pull_request" => on.pull_request = Some(EventFilter::default()),
                "workflow_dispatch" => on.workflow_dispatch = Some(true),
                _ => {}
            }
        }
    }
}
