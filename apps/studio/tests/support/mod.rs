//! Own the complete Windows fixture process tree, including browser children.
use harness_core::process_job::ProcessJob;
use std::{
    ops::{Deref, DerefMut},
    process::Child,
};

pub struct FixtureProcess {
    child: Child,
    job: ProcessJob,
}

impl FixtureProcess {
    pub fn new(mut child: Child) -> Self {
        let job = match ProcessJob::attach(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("Cannot own fixture process tree: {error}");
            }
        };
        Self { child, job }
    }
}

impl Deref for FixtureProcess {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}
impl DerefMut for FixtureProcess {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}
impl Drop for FixtureProcess {
    fn drop(&mut self) {
        self.job.terminate();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
