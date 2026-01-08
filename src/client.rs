//! Telnet client for connecting to the Reverse Beacon Network.
//!
//! This module handles the TCP connection to the RBN telnet server,
//! including login and streaming of spot data.

use anyhow::{Context, Result};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

/// Default RBN telnet server for CW/RTTY spots.
pub const RBN_HOST: &str = "telnet.reversebeacon.net";

/// Default port for CW/RTTY spots.
pub const RBN_PORT_CW: u16 = 7000;

/// Default port for FT8 spots.
pub const RBN_PORT_FT8: u16 = 7001;

/// Configuration for the RBN client.
#[derive(Debug, Clone)]
pub struct RbnClientConfig {
    /// Hostname of the RBN server.
    pub host: String,

    /// Port number.
    pub port: u16,

    /// Callsign to use for login.
    pub callsign: String,

    /// Connection timeout.
    pub connect_timeout: Duration,

    /// Read timeout for individual lines.
    pub read_timeout: Duration,

    /// Whether to automatically reconnect on disconnect.
    pub auto_reconnect: bool,

    /// Delay between reconnection attempts.
    pub reconnect_delay: Duration,
}

impl Default for RbnClientConfig {
    fn default() -> Self {
        Self {
            host: RBN_HOST.to_string(),
            port: RBN_PORT_CW,
            callsign: "N0CALL".to_string(),
            connect_timeout: Duration::from_secs(30),
            read_timeout: Duration::from_secs(120),
            auto_reconnect: true,
            reconnect_delay: Duration::from_secs(5),
        }
    }
}

impl RbnClientConfig {
    /// Create a new configuration with the given callsign.
    pub fn with_callsign(callsign: impl Into<String>) -> Self {
        Self {
            callsign: callsign.into(),
            ..Default::default()
        }
    }

    /// Set the host and port.
    pub fn with_server(mut self, host: impl Into<String>, port: u16) -> Self {
        self.host = host.into();
        self.port = port;
        self
    }
}

/// Events from the RBN client.
#[derive(Debug)]
pub enum RbnEvent {
    /// A line was received from the server.
    Line(String),

    /// Connection was established.
    Connected,

    /// Connection was lost.
    Disconnected(String),

    /// An error occurred.
    Error(String),
}

/// Async RBN telnet client.
pub struct RbnClient {
    config: RbnClientConfig,
}

impl RbnClient {
    /// Create a new RBN client with the given configuration.
    pub fn new(config: RbnClientConfig) -> Self {
        Self { config }
    }

    /// Connect to the RBN server and start streaming spots.
    ///
    /// Returns a receiver channel that will receive `RbnEvent`s.
    /// The connection runs in a background task.
    pub async fn connect(self) -> Result<mpsc::Receiver<RbnEvent>> {
        let (tx, rx) = mpsc::channel(1000);

        tokio::spawn(async move {
            self.run_connection_loop(tx).await;
        });

        Ok(rx)
    }

    /// Run the main connection loop with auto-reconnect.
    async fn run_connection_loop(self, tx: mpsc::Sender<RbnEvent>) {
        loop {
            match self.connect_and_stream(&tx).await {
                Ok(()) => {
                    info!("Connection closed normally");
                }
                Err(e) => {
                    error!("Connection error: {}", e);
                    let _ = tx.send(RbnEvent::Error(e.to_string())).await;
                }
            }

            let _ = tx
                .send(RbnEvent::Disconnected("Connection lost".to_string()))
                .await;

            if !self.config.auto_reconnect {
                break;
            }

            info!(
                "Reconnecting in {} seconds...",
                self.config.reconnect_delay.as_secs()
            );
            tokio::time::sleep(self.config.reconnect_delay).await;
        }
    }

    /// Connect to the server and stream lines until disconnected.
    async fn connect_and_stream(&self, tx: &mpsc::Sender<RbnEvent>) -> Result<()> {
        let addr = format!("{}:{}", self.config.host, self.config.port);
        info!("Connecting to {}...", addr);

        // Connect with timeout
        let stream = timeout(self.config.connect_timeout, TcpStream::connect(&addr))
            .await
            .context("Connection timeout")?
            .context("Failed to connect")?;

        info!("Connected to {}", addr);

        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line_buf = String::with_capacity(256);

        // Wait for login prompt and send callsign
        // The RBN server typically sends some welcome text and then expects a callsign
        let mut login_sent = false;
        let mut welcome_lines = 0;

        loop {
            line_buf.clear();

            let read_result =
                timeout(self.config.read_timeout, reader.read_line(&mut line_buf)).await;

            match read_result {
                Ok(Ok(0)) => {
                    // EOF - connection closed
                    return Ok(());
                }
                Ok(Ok(_n)) => {
                    let line = line_buf.trim_end();
                    debug!("Received: {}", line);

                    // Handle login sequence
                    if !login_sent {
                        welcome_lines += 1;

                        // Look for callsign prompt or just send after a few lines
                        if line.contains("call:")
                            || line.contains("callsign")
                            || line.contains("login")
                            || welcome_lines >= 3
                        {
                            info!("Sending callsign: {}", self.config.callsign);
                            writer
                                .write_all(format!("{}\r\n", self.config.callsign).as_bytes())
                                .await
                                .context("Failed to send callsign")?;
                            writer.flush().await?;
                            login_sent = true;
                            let _ = tx.send(RbnEvent::Connected).await;
                        }
                    } else {
                        // After login, forward all lines
                        if tx.send(RbnEvent::Line(line.to_string())).await.is_err() {
                            // Receiver dropped
                            return Ok(());
                        }
                    }
                }
                Ok(Err(e)) => {
                    return Err(e).context("Read error");
                }
                Err(_) => {
                    warn!("Read timeout, connection may be stale");
                    return Err(anyhow::anyhow!("Read timeout"));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RbnClientConfig::default();
        assert_eq!(config.host, RBN_HOST);
        assert_eq!(config.port, RBN_PORT_CW);
        assert!(config.auto_reconnect);
    }

    #[test]
    fn test_config_builder() {
        let config = RbnClientConfig::with_callsign("W6JSV").with_server("test.example.com", 1234);

        assert_eq!(config.callsign, "W6JSV");
        assert_eq!(config.host, "test.example.com");
        assert_eq!(config.port, 1234);
    }
}
