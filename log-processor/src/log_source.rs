use crate::{ProcessorError, Result};
use futures::stream::Stream;
use std::path::PathBuf;
use std::pin::Pin;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{debug, error, info};

pub enum LogSource {
  File(PathBuf),
  Command(Vec<String>),
}

impl LogSource {
  pub fn from_file(path: PathBuf) -> Self {
    Self::File(path)
  }

  pub fn from_command(command: Vec<String>) -> Self {
    Self::Command(command)
  }

  /// Create a stream of log lines from this source
  pub async fn into_stream(
    self,
  ) -> Result<Pin<Box<dyn Stream<Item = Result<String>> + Send>>> {
    match self {
      LogSource::File(path) => {
        info!("Opening log file: {:?}", path);
        let file = tokio::fs::File::open(&path).await?;

        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        let stream = async_stream::stream! {
            while let Some(line) = lines.next_line().await.transpose() {
                match line {
                    Ok(line) => {
                        debug!("Read line from file: {}", line);
                        yield Ok(line);
                    }
                    Err(e) => {
                        error!("Error reading from file: {}", e);
                        yield Err(ProcessorError::IoError(e));
                        break;
                    }
                }
            }

            info!("File stream ended");
        };

        Ok(Box::pin(stream))
      }
      LogSource::Command(args) => {
        if args.is_empty() {
          return Err(ProcessorError::InvalidLogSource(
            "Command cannot be empty".to_string(),
          ));
        }

        info!("Starting command: {}", args.join(" "));

        let mut child = Command::new(&args[0])
          .args(&args[1..])
          .stdout(std::process::Stdio::piped())
          .stderr(std::process::Stdio::piped())
          .spawn()?;

        let stdout = child.stdout.take().ok_or_else(|| {
          ProcessorError::InvalidLogSource(
            "Failed to capture command stdout".to_string(),
          )
        })?;

        info!("Command started successfully, reading stdout");

        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();

        let stream = async_stream::stream! {
            while let Some(line) = lines.next_line().await.transpose() {
                match line {
                    Ok(line) => {
                        debug!("Read line from command: {}", line);
                        yield Ok(line);
                    }
                    Err(e) => {
                        error!("Error reading from command: {}", e);
                        yield Err(ProcessorError::IoError(e));
                        break;
                    }
                }
            }

            info!("Command stream ended");
        };

        Ok(Box::pin(stream))
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use futures::StreamExt;

  #[tokio::test]
  async fn test_command_source() {
    // Test with a simple echo command
    let source = LogSource::from_command(vec![
      "echo".to_string(),
      "test line".to_string(),
    ]);

    let mut stream = source.into_stream().await.unwrap();

    if let Some(Ok(line)) = stream.next().await {
      assert_eq!(line, "test line");
    } else {
      panic!("Expected to read a line");
    }
  }
}
