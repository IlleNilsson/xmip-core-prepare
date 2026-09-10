#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt;
use stream::Stream;

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

#[cfg(test)]
mod tests {
    use super::*;
    use xcore::StreamId;

    struct Upper;
    struct Reverse;
    struct Refuse;

    impl PrepareStep for Upper {
        fn name(&self) -> &'static str {
            "upper"
        }

        fn execute(&self, input: &Stream) -> Result<Stream, PrepareError> {
            Ok(Stream::new(
                StreamId::new(1),
                input.bytes().to_ascii_uppercase(),
                input.media_type().map(str::to_string),
            ))
        }
    }

    impl PrepareStep for Reverse {
        fn name(&self) -> &'static str {
            "reverse"
        }

        fn execute(&self, input: &Stream) -> Result<Stream, PrepareError> {
            let mut bytes = input.bytes().to_vec();
            bytes.reverse();
            Ok(Stream::new(StreamId::new(1), bytes, None))
        }
    }

    impl PrepareStep for Refuse {
        fn name(&self) -> &'static str {
            "refuse"
        }

        fn execute(&self, _: &Stream) -> Result<Stream, PrepareError> {
            Err(PrepareError {
                message: "not this one".to_string(),
            })
        }
    }

    #[test]
    fn steps_run_in_order_over_the_stream() {
        let pipeline = PreparePipeline::new(vec![Box::new(Upper), Box::new(Reverse)]);
        let input = Stream::new(StreamId::new(1), b"abc".to_vec(), None);
        assert_eq!(pipeline.execute(&input).expect("prepared").bytes(), b"CBA");
        assert_eq!(input.bytes(), b"abc", "the input is untouched");
    }

    #[test]
    fn a_refusing_step_stops_the_pipeline_with_its_reason() {
        let pipeline = PreparePipeline::new(vec![Box::new(Upper), Box::new(Refuse)]);
        let input = Stream::new(StreamId::new(1), b"abc".to_vec(), None);
        let error = pipeline.execute(&input).expect_err("refused");
        assert_eq!(error.to_string(), "not this one");
        assert_eq!(Refuse.name(), "refuse");
    }
}
