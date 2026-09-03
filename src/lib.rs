#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt;
use xmip_stream::Stream;

#[derive(Debug)]
pub struct PrepareError {
    pub message: String,
}

impl fmt::Display for PrepareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for PrepareError {}

pub trait PrepareStep: Send + Sync {
    fn name(&self) -> &'static str;
    fn execute(&self, input: &Stream) -> Result<Stream, PrepareError>;
}

pub struct PreparePipeline {
    steps: Vec<Box<dyn PrepareStep>>,
}

impl PreparePipeline {
    pub fn new(steps: Vec<Box<dyn PrepareStep>>) -> Self {
        Self { steps }
    }

    pub fn execute(&self, input: &Stream) -> Result<Stream, PrepareError> {
        let mut current = input.clone();
        for step in &self.steps {
            current = step.execute(&current)?;
        }
        Ok(current)
    }
}
