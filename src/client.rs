//! Telnet client for connecting to the Reverse Beacon Network.
//!
//! This module handles the TCP connection to the RBN telnet server,
//! including login and streaming of spot data.

use anyhow::{Context, Result};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
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

        // Phase 1: Handle login sequence
        // The server sends "Please enter your call: " without a trailing newline,
        // so we need to read bytes until we see the prompt
        self.handle_login(&mut reader, &mut writer).await?;
        let _ = tx.send(RbnEvent::Connected).await;

        // Phase 2: Stream spot lines
        let mut line_buf = String::with_capacity(256);

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

                    if tx.send(RbnEvent::Line(line.to_string())).await.is_err() {
                        // Receiver dropped
                        return Ok(());
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

    /// Handle the login sequence by reading bytes until we see the callsign prompt.
    async fn handle_login<R, W>(&self, reader: &mut R, writer: &mut W) -> Result<()>
    where
        R: AsyncReadExt + Unpin,
        W: AsyncWriteExt + Unpin,
    {
        let mut buf = Vec::with_capacity(1024);
        let mut byte = [0u8; 1];

        // Read until we see "call:" prompt (case-insensitive)
        loop {
            let read_result = timeout(self.config.connect_timeout, reader.read(&mut byte)).await;

            match read_result {
                Ok(Ok(0)) => {
                    return Err(anyhow::anyhow!("Connection closed during login"));
                }
                Ok(Ok(_)) => {
                    buf.push(byte[0]);

                    // Check if we've received the login prompt
                    // Looking for "call:" which appears in "Please enter your call:"
                    if buf.len() >= 5 {
                        let tail = String::from_utf8_lossy(&buf[buf.len() - 5..]);
                        if tail.eq_ignore_ascii_case("call:") {
                            debug!(
                                "Login prompt received: {}",
                                String::from_utf8_lossy(&buf).trim()
                            );
                            break;
                        }
                    }

                    // Safety limit to avoid reading forever
                    if buf.len() > 4096 {
                        return Err(anyhow::anyhow!("No login prompt found in initial data"));
                    }
                }
                Ok(Err(e)) => {
                    return Err(e).context("Read error during login");
                }
                Err(_) => {
                    return Err(anyhow::anyhow!("Timeout waiting for login prompt"));
                }
            }
        }

        // Send callsign
        info!("Sending callsign: {}", self.config.callsign);
        writer
            .write_all(format!("{}\r\n", self.config.callsign).as_bytes())
            .await
            .context("Failed to send callsign")?;
        writer.flush().await?;

        // Read the post-login messages until we see the command prompt (ends with ">")
        // e.g., "W6JSV de RELAY 08-Jan-2026 03:13Z >"
        buf.clear();
        loop {
            let read_result = timeout(self.config.connect_timeout, reader.read(&mut byte)).await;

            match read_result {
                Ok(Ok(0)) => {
                    return Err(anyhow::anyhow!("Connection closed after login"));
                }
                Ok(Ok(_)) => {
                    buf.push(byte[0]);

                    // Check for command prompt ending with ">"
                    if byte[0] == b'>' {
                        let response = String::from_utf8_lossy(&buf);
                        for line in response.lines() {
                            let trimmed = line.trim();
                            if !trimmed.is_empty() {
                                debug!("Login response: {}", trimmed);
                            }
                        }
                        info!("Login complete");
                        return Ok(());
                    }

                    // Safety limit
                    if buf.len() > 4096 {
                        // Assume login succeeded if we got this far
                        debug!("No command prompt found, assuming login succeeded");
                        return Ok(());
                    }
                }
                Ok(Err(e)) => {
                    return Err(e).context("Read error after login");
                }
                Err(_) => {
                    // Timeout is OK here - server might not send a prompt
                    debug!("Timeout after login, assuming success");
                    return Ok(());
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
