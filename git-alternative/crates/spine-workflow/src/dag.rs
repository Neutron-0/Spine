use crate::ast::WorkflowSpec;
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum DagError {
    #[error("Unknown job dependency: job '{job}' depends on undefined job '{depends_on}'")]
    MissingDependency { job: String, depends_on: String },
    #[error("Cyclic job dependency detected in workflow: {0}")]
    CycleDetected(String),
}

#[derive(Clone, Debug)]
pub struct JobDag {
    pub jobs: HashSet<String>,
    pub dependencies: HashMap<String, HashSet<String>>, // job -> set of jobs it depends on
    pub dependents: HashMap<String, HashSet<String>>,   // job -> set of jobs that depend on it
}

impl JobDag {
    pub fn build(spec: &WorkflowSpec) -> Result<Self, DagError> {
        let mut jobs = HashSet::new();
        let mut dependencies = HashMap::new();
        let mut dependents = HashMap::new();

        for job_id in spec.jobs.keys() {
            jobs.insert(job_id.clone());
            dependencies.insert(job_id.clone(), HashSet::new());
            dependents.insert(job_id.clone(), HashSet::new());
        }

        for (job_id, job_spec) in &spec.jobs {
            for needed in &job_spec.needs {
                if !jobs.contains(needed) {
                    return Err(DagError::MissingDependency {
                        job: job_id.clone(),
                        depends_on: needed.clone(),
                    });
                }
                dependencies.get_mut(job_id).unwrap().insert(needed.clone());
                dependents.get_mut(needed).unwrap().insert(job_id.clone());
            }
        }

        let dag = Self {
            jobs,
            dependencies,
            dependents,
        };

        dag.detect_cycles()?;
        Ok(dag)
    }

    fn detect_cycles(&self) -> Result<(), DagError> {
        let mut visited = HashSet::new();
        let mut recursion_stack = HashSet::new();
        let mut cycle_path = Vec::new();

        for job in &self.jobs {
            if !visited.contains(job)
                && self.dfs_cycle(job, &mut visited, &mut recursion_stack, &mut cycle_path)
            {
                cycle_path.reverse();
                return Err(DagError::CycleDetected(cycle_path.join(" -> ")));
            }
        }

        Ok(())
    }

    fn dfs_cycle(
        &self,
        node: &str,
        visited: &mut HashSet<String>,
        recursion_stack: &mut HashSet<String>,
        path: &mut Vec<String>,
    ) -> bool {
        visited.insert(node.to_string());
        recursion_stack.insert(node.to_string());
        path.push(node.to_string());

        if let Some(deps) = self.dependencies.get(node) {
            for dep in deps {
                if !visited.contains(dep) {
                    if self.dfs_cycle(dep, visited, recursion_stack, path) {
                        return true;
                    }
                } else if recursion_stack.contains(dep) {
                    path.push(dep.to_string());
                    return true;
                }
            }
        }

        recursion_stack.remove(node);
        path.pop();
        false
    }

    /// Computes topological sort: returns jobs in dependency execution order.
    pub fn topological_order(&self) -> Vec<String> {
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        for job in &self.jobs {
            let deg = self.dependencies.get(job).map(|d| d.len()).unwrap_or(0);
            in_degree.insert(job.clone(), deg);
        }

        let mut queue: Vec<String> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(job, _)| job.clone())
            .collect();
        queue.sort(); // deterministic ordering

        let mut order = Vec::new();

        while let Some(job) = queue.pop() {
            order.push(job.clone());
            if let Some(children) = self.dependents.get(&job) {
                let mut sorted_children: Vec<_> = children.iter().collect();
                sorted_children.sort();
                for child in sorted_children {
                    if let Some(deg) = in_degree.get_mut(child) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push(child.clone());
                        }
                    }
                }
            }
        }

        order
    }

    /// Returns the list of jobs that are ready to run given the set of completed jobs.
    pub fn ready_jobs(
        &self,
        completed_successful_jobs: &HashSet<String>,
        in_progress_or_done_jobs: &HashSet<String>,
    ) -> Vec<String> {
        let mut ready = Vec::new();
        for job in &self.jobs {
            if in_progress_or_done_jobs.contains(job) {
                continue;
            }
            let deps = self.dependencies.get(job).unwrap();
            if deps.iter().all(|d| completed_successful_jobs.contains(d)) {
                ready.push(job.clone());
            }
        }
        ready.sort();
        ready
    }
}
