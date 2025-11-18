use anyhow::{Ok, Result};
use common::{run_rathole_client, PING, PONG};
use rand::Rng;
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
    sync::broadcast,
    time,
};
use tracing::{debug, info, instrument};
use tracing_subscriber::EnvFilter;

use crate::common::run_rathole_server;

mod common;

const ECHO_SERVER_ADDR: &str = "127.0.0.1:8080";
const PINGPONG_SERVER_ADDR: &str = "127.0.0.1:8081";
const HITTER_NUM: usize = 4;

#[derive(Clone, Copy, Debug)]
enum Type {
    Tcp,
    Udp,
}

fn init() {
    let level = "info";
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::from(level)),
        )
        .try_init();
}

use lazy_static::lazy_static;
use tokio::sync::Mutex;

lazy_static! {
    static ref TEST_MUTEX: Mutex<()> = Mutex::new(());
}

#[tokio::test]
async fn tcp() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;
    init();

    // Spawn a echo server
    tokio::spawn(async move {
        if let Err(e) = common::tcp::echo_server(ECHO_SERVER_ADDR).await {
            panic!("Failed to run the echo server for testing: {:?}", e);
        }
    });

    // Spawn a pingpong server
    tokio::spawn(async move {
        if let Err(e) = common::tcp::pingpong_server(PINGPONG_SERVER_ADDR).await {
            panic!("Failed to run the pingpong server for testing: {:?}", e);
        }
    });

    test(
        "tests/for_tcp/tcp_transport.toml",
        Type::Tcp,
        "127.0.0.1:2334",
        "127.0.0.1:2335",
    )
    .await?;

    #[cfg(any(
        // FIXME: Self-signed certificate on macOS nativetls requires manual interference.
        all(target_os = "macos", feature = "rustls"),
        // On other OS accept run with either
        all(
            not(target_os = "macos"),
            any(feature = "native-tls", feature = "rustls")
        ),
    ))]
    test(
        "tests/for_tcp/tls_transport.toml",
        Type::Tcp,
        "127.0.0.1:2334",
        "127.0.0.1:2335",
    )
    .await?;

    #[cfg(feature = "noise")]
    test(
        "tests/for_tcp/noise_transport.toml",
        Type::Tcp,
        "127.0.0.1:2334",
        "127.0.0.1:2335",
    )
    .await?;

    #[cfg(any(feature = "websocket-native-tls", feature = "websocket-rustls"))]
    test(
        "tests/for_tcp/websocket_transport.toml",
        Type::Tcp,
        "127.0.0.1:2334",
        "127.0.0.1:2335",
    )
    .await?;

    #[cfg(not(target_os = "macos"))]
    #[cfg(any(feature = "websocket-native-tls", feature = "websocket-rustls"))]
    test(
        "tests/for_tcp/websocket_tls_transport.toml",
        Type::Tcp,
        "127.0.0.1:2334",
        "127.0.0.1:2335",
    )
    .await?;

    Ok(())
}

#[tokio::test]
async fn udp() -> Result<()> {
    let _guard = TEST_MUTEX.lock().await;
    init();

    // Spawn a echo server
    tokio::spawn(async move {
        if let Err(e) = common::udp::echo_server("127.0.0.1:8082").await {
            panic!("Failed to run the echo server for testing: {:?}", e);
        }
    });

    // Spawn a pingpong server
    tokio::spawn(async move {
        if let Err(e) = common::udp::pingpong_server("127.0.0.1:8083").await {
            panic!("Failed to run the pingpong server for testing: {:?}", e);
        }
    });

    test(
        "tests/for_udp/tcp_transport.toml",
        Type::Udp,
        "127.0.0.1:2344",
        "127.0.0.1:2345",
    )
    .await?;

    #[cfg(any(
        // FIXME: Self-signed certificate on macOS nativetls requires manual interference.
        all(target_os = "macos", feature = "rustls"),
        // On other OS accept run with either
        all(
            not(target_os = "macos"),
            any(feature = "native-tls", feature = "rustls")
        ),
    ))]
    test(
        "tests/for_udp/tls_transport.toml",
        Type::Udp,
        "127.0.0.1:2344",
        "127.0.0.1:2345",
    )
    .await?;

    #[cfg(feature = "noise")]
    test(
        "tests/for_udp/noise_transport.toml",
        Type::Udp,
        "127.0.0.1:2344",
        "127.0.0.1:2345",
    )
    .await?;

    #[cfg(any(feature = "websocket-native-tls", feature = "websocket-rustls"))]
    test(
        "tests/for_udp/websocket_transport.toml",
        Type::Udp,
        "127.0.0.1:2344",
        "127.0.0.1:2345",
    )
    .await?;

    #[cfg(not(target_os = "macos"))]
    #[cfg(any(feature = "websocket-native-tls", feature = "websocket-rustls"))]
    test(
        "tests/for_udp/websocket_tls_transport.toml",
        Type::Udp,
        "127.0.0.1:2344",
        "127.0.0.1:2345",
    )
    .await?;

    Ok(())
}

#[instrument]
async fn test(
    config_path: &'static str,
    t: Type,
    echo_addr: &'static str,
    pingpong_addr: &'static str,
) -> Result<()> {
    if cfg!(not(all(feature = "client", feature = "server"))) {
        // Skip the test if the client or the server is not enabled
        return Ok(());
    }

    let (client_shutdown_tx, client_shutdown_rx) = broadcast::channel(1);
    let (server_shutdown_tx, server_shutdown_rx) = broadcast::channel(1);

    // Start the client
    info!("start the client");
    let client_shutdown_tx_clone = client_shutdown_tx.clone();
    let client = tokio::spawn(async move {
        run_rathole_client(config_path, client_shutdown_rx, client_shutdown_tx_clone)
            .await
            .unwrap();
    });
    info!("client started");

    // Sleep for 1 second. Expect the client keep retrying to reach the server
    time::sleep(Duration::from_secs(1)).await;

    // Start the server
    info!("start the server");
    let server_shutdown_tx_clone = server_shutdown_tx.clone();
    let server = tokio::spawn(async move {
        run_rathole_server(config_path, server_shutdown_rx, server_shutdown_tx_clone)
            .await
            .unwrap();
    });
    time::sleep(Duration::from_secs(5)).await; // Wait for the client to retry

    info!("echo");
    echo_hitter(echo_addr, t).await.unwrap();
    info!("pingpong");
    pingpong_hitter(pingpong_addr, t).await.unwrap();

    // Simulate the client crash and restart
    info!("shutdown the client");
    client_shutdown_tx.send(true)?;
    let _ = tokio::join!(client);
    info!("client shutdown");

    info!("restart the client");
    let client_shutdown_rx = client_shutdown_tx.subscribe();
    let client_shutdown_tx_clone = client_shutdown_tx.clone();
    let client = tokio::spawn(async move {
        run_rathole_client(config_path, client_shutdown_rx, client_shutdown_tx_clone)
            .await
            .unwrap();
    });
    info!("client restarted");
    time::sleep(Duration::from_secs(1)).await; // Wait for the client to start

    info!("echo");
    echo_hitter(echo_addr, t).await.unwrap();
    info!("pingpong");
    pingpong_hitter(pingpong_addr, t).await.unwrap();

    // Simulate the server crash and restart
    info!("shutdown the server");
    server_shutdown_tx.send(true)?;
    let _ = tokio::join!(server);
    info!("server shutdown");

    info!("restart the server");
    let server_shutdown_rx = server_shutdown_tx.subscribe();
    let server_shutdown_tx_clone = server_shutdown_tx.clone();
    let server = tokio::spawn(async move {
        run_rathole_server(config_path, server_shutdown_rx, server_shutdown_tx_clone)
            .await
            .unwrap();
    });
    info!("server restarted");
    time::sleep(Duration::from_millis(2500)).await; // Wait for the client to retry

    // Simulate heavy load
    info!("lots of echo and pingpong");

    let mut v = Vec::new();

    for _ in 0..HITTER_NUM / 2 {
        v.push(tokio::spawn(async move {
            echo_hitter(echo_addr, t).await.unwrap();
        }));

        v.push(tokio::spawn(async move {
            pingpong_hitter(pingpong_addr, t).await.unwrap();
        }));
    }

    for h in v {
        assert!(tokio::join!(h).0.is_ok());
    }

    // Shutdown
    info!("shutdown the server and the client");
    server_shutdown_tx.send(true)?;
    client_shutdown_tx.send(true)?;

    let _ = tokio::join!(server, client);

    Ok(())
}

async fn echo_hitter(addr: &'static str, t: Type) -> Result<()> {
    match t {
        Type::Tcp => tcp_echo_hitter(addr).await,
        Type::Udp => udp_echo_hitter(addr).await,
    }
}

async fn pingpong_hitter(addr: &'static str, t: Type) -> Result<()> {
    match t {
        Type::Tcp => tcp_pingpong_hitter(addr).await,
        Type::Udp => udp_pingpong_hitter(addr).await,
    }
}

async fn tcp_echo_hitter(addr: &'static str) -> Result<()> {
    let mut conn = TcpStream::connect(addr).await?;

    let mut wr = [0u8; 1024];
    let mut rd = [0u8; 1024];
    for _ in 0..100 {
        rand::thread_rng().fill(&mut wr);
        conn.write_all(&wr).await?;
        conn.read_exact(&mut rd).await?;
        assert_eq!(wr, rd);
    }

    Ok(())
}

async fn udp_echo_hitter(addr: &'static str) -> Result<()> {
    let conn = UdpSocket::bind("127.0.0.1:0").await?;
    conn.connect(addr).await?;

    let mut wr = [0u8; 128];
    let mut rd = [0u8; 128];
    for _ in 0..3 {
        rand::thread_rng().fill(&mut wr);

        conn.send(&wr).await?;
        debug!("send");

        conn.recv(&mut rd).await?;
        debug!("recv");

        assert_eq!(wr, rd);
    }
    Ok(())
}

async fn tcp_pingpong_hitter(addr: &'static str) -> Result<()> {
    let mut conn = TcpStream::connect(addr).await?;

    let wr = PING.as_bytes();
    let mut rd = [0u8; PONG.len()];

    for _ in 0..100 {
        conn.write_all(wr).await?;
        conn.read_exact(&mut rd).await?;
        assert_eq!(rd, PONG.as_bytes());
    }

    Ok(())
}

async fn udp_pingpong_hitter(addr: &'static str) -> Result<()> {
    let conn = UdpSocket::bind("127.0.0.1:0").await?;
    conn.connect(&addr).await?;

    let wr = PING.as_bytes();
    let mut rd = [0u8; PONG.len()];

    for _ in 0..3 {
        conn.send(wr).await?;
        debug!("ping");

        conn.recv(&mut rd).await?;
        debug!("pong");

        assert_eq!(rd, PONG.as_bytes());
    }

    Ok(())
}
