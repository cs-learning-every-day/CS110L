mod request;
mod response;

use std::collections::HashMap;
use std::collections::HashSet;
use std::io::Error;
use std::io::ErrorKind;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use rand::seq::IteratorRandom;
use rand::SeedableRng;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::sync::RwLock;
use tokio::time::sleep;

/// Contains information parsed from the command-line invocation of balancebeam. The Clap macros
/// provide a fancy way to automatically construct a command-line argument parser.
#[derive(Parser, Debug)]
#[command(about = "Fun with load balancing")]
struct CmdOptions {
    #[arg(
        short,
        long,
        help = "IP/port to bind to",
        default_value = "0.0.0.0:1100"
    )]
    bind: String,

    #[arg(short, long, help = "Upstream host to forward requests to")]
    upstream: Vec<String>,

    #[arg(
        long,
        help = "Perform active health checks on this interval (in seconds)",
        default_value = "10"
    )]
    active_health_check_interval: usize,

    #[arg(
        long,
        help = "Path to send request to for active health checks",
        default_value = "/"
    )]
    active_health_check_path: String,

    #[arg(
        long,
        help = "Maximum number of requests to accept per IP per minute (0 = unlimited)",
        default_value = "0"
    )]
    max_requests_per_minute: usize,
}

/// Contains information about the state of balancebeam (e.g. what servers we are currently proxying
/// to, what servers have failed, rate limiting counts, etc.)
///
/// You should add fields to this struct in later milestones.
#[derive(Clone)]
struct ProxyState {
    /// How frequently we check whether upstream servers are alive (Milestone 4)
    #[allow(dead_code)]
    active_health_check_interval: usize,
    /// Where we should send requests when doing active health checks (Milestone 4)
    #[allow(dead_code)]
    active_health_check_path: String,
    /// Maximum number of requests an individual IP can make in a minute (Milestone 5)
    #[allow(dead_code)]
    max_requests_per_minute: usize,
    /// Addresses of servers that we are proxying to
    upstream_addresses: Vec<String>,
    // Alive of upstream
    alive_upstreams: Arc<RwLock<HashSet<String>>>,
    // Rate limit
    rate_limit_map: Arc<Mutex<HashMap<String, u32>>>,
}

#[tokio::main]
async fn main() {
    // Initialize the logging library. You can print log messages using the `log` macros:
    // https://docs.rs/log/0.4.8/log/ You are welcome to continue using print! statements; this
    // just looks a little prettier.
    if let Err(_) = std::env::var("RUST_LOG") {
        std::env::set_var("RUST_LOG", "debug");
    }
    pretty_env_logger::init();

    // Parse the command line arguments passed to this program
    let options = CmdOptions::parse();
    if options.upstream.len() < 1 {
        log::error!("At least one upstream server must be specified using the --upstream option.");
        std::process::exit(1);
    }

    // Start listening for connections
    let listener = match TcpListener::bind(&options.bind).await {
        Ok(listener) => listener,
        Err(err) => {
            log::error!("Could not bind to {}: {}", options.bind, err);
            std::process::exit(1);
        }
    };
    log::info!("Listening for requests on {}", options.bind);

    // Handle incoming connections
    let hashd_upstreams = options.upstream.clone().into_iter().collect();
    let state = ProxyState {
        upstream_addresses: options.upstream,
        active_health_check_interval: options.active_health_check_interval,
        active_health_check_path: options.active_health_check_path,
        max_requests_per_minute: options.max_requests_per_minute,
        alive_upstreams: Arc::new(RwLock::new(hashd_upstreams)),
        rate_limit_map: Arc::new(Mutex::new(HashMap::new())),
    };

    let tmp_state = state.clone();
    tokio::spawn(async move {
        health_check(&tmp_state).await;
    });

    let tmp_state = state.clone();
    tokio::spawn(async move {
        ramte_limit_map_clear(&tmp_state).await;
    });

    loop {
        if let Ok((stream, _)) = listener.accept().await {
            let state = state.clone();
            // Handle the connection!
            tokio::spawn(async move {
                handle_connection(stream, &state).await;
            });
        }
    }
}

async fn ramte_limit_map_clear(state: &ProxyState) {
    loop {
        sleep(Duration::from_secs(60)).await;
        let mut rate_limit_map = state.rate_limit_map.clone().lock_owned().await;
        rate_limit_map.clear();
    }
}

async fn health_check(state: &ProxyState) {
    loop {
        sleep(Duration::from_secs(
            state.active_health_check_interval.try_into().unwrap(),
        ))
        .await;

        let mut alive_upstreams = state.alive_upstreams.write().await;
        alive_upstreams.clear();

        for upstream_ip in &state.upstream_addresses {
            let req = http::Request::builder()
                .method(http::Method::GET)
                .uri(&state.active_health_check_path)
                .header("Host", upstream_ip)
                .body(Vec::new())
                .unwrap();

            match TcpStream::connect(upstream_ip).await {
                Ok(mut stream) => {
                    if let Err(err) = request::write_to_stream(&req, &mut stream).await {
                        log::error!(
                            "Failed to send request to upstream {}: {}",
                            upstream_ip,
                            err
                        );
                        continue;
                    }

                    match response::read_from_stream(&mut stream, &req.method()).await {
                        Ok(response) => match response.status().as_u16() {
                            200 => {
                                alive_upstreams.insert(upstream_ip.to_string());
                            }
                            status @ _ => {
                                log::error!(
                                    "health check upstream server: {} : {}",
                                    upstream_ip,
                                    status
                                );
                            }
                        },
                        Err(error) => {
                            log::error!("Error read from stream {:?}", error);
                            continue;
                        }
                    }
                }
                Err(err) => {
                    log::error!("Failed to connect to upstream {}: {}", upstream_ip, err);
                    continue;
                }
            }
        }
    }
}

async fn connect_to_upstream(state: &ProxyState) -> Result<TcpStream, std::io::Error> {
    let mut rng = rand::rngs::StdRng::from_entropy();
    loop {
        let alive_upstreams = state.alive_upstreams.read().await;

        if let Some(upstream_ip) = alive_upstreams.clone().iter().choose(&mut rng) {
            drop(alive_upstreams);

            match TcpStream::connect(upstream_ip).await {
                Ok(stream) => return Ok(stream),
                Err(err) => {
                    log::error!("Failed to connect to upstream {}: {}", upstream_ip, err);

                    let mut alive_upstreams = state.alive_upstreams.write().await;
                    alive_upstreams.remove(upstream_ip);

                    if alive_upstreams.len() == 0 {
                        log::error!("Failed to connect to upstream: empty alive_upstreams");
                        return Err(err);
                    }
                }
            }
        } else {
            log::error!("Failed to connect to upstream: empty alive_upstreams");
            return Err(Error::new(ErrorKind::Other, "empty alive_upstreams"));
        }
    }
}

async fn send_response(client_conn: &mut TcpStream, response: &http::Response<Vec<u8>>) {
    let client_ip = client_conn.peer_addr().unwrap().ip().to_string();
    log::info!(
        "{} <- {}",
        client_ip,
        response::format_response_line(&response)
    );
    if let Err(error) = response::write_to_stream(&response, client_conn).await {
        log::warn!("Failed to send response to client: {}", error);
        return;
    }
}

async fn handle_connection(mut client_conn: TcpStream, state: &ProxyState) {
    let client_ip = client_conn.peer_addr().unwrap().ip().to_string();
    log::info!("Connection received from {}", client_ip);

    // Open a connection to a random destination server
    let mut upstream_conn = match connect_to_upstream(state).await {
        Ok(stream) => stream,
        Err(_error) => {
            let response = response::make_http_error(http::StatusCode::BAD_GATEWAY);
            send_response(&mut client_conn, &response).await;
            return;
        }
    };
    let upstream_ip = upstream_conn.peer_addr().unwrap().ip().to_string();

    // The client may now send us one or more requests. Keep trying to read requests until the
    // client hangs up or we get an error.
    loop {
        // Read a request from the client
        let mut request = match request::read_from_stream(&mut client_conn).await {
            Ok(request) => request,
            // Handle case where client closed connection and is no longer sending requests
            Err(request::Error::IncompleteRequest(0)) => {
                log::debug!("Client finished sending requests. Shutting down connection");
                return;
            }
            // Handle I/O error in reading from the client
            Err(request::Error::ConnectionError(io_err)) => {
                log::info!("Error reading request from client stream: {}", io_err);
                return;
            }
            Err(error) => {
                log::debug!("Error parsing request: {:?}", error);
                let response = response::make_http_error(match error {
                    request::Error::IncompleteRequest(_)
                    | request::Error::MalformedRequest(_)
                    | request::Error::InvalidContentLength
                    | request::Error::ContentLengthMismatch => http::StatusCode::BAD_REQUEST,
                    request::Error::RequestBodyTooLarge => http::StatusCode::PAYLOAD_TOO_LARGE,
                    request::Error::ConnectionError(_) => http::StatusCode::SERVICE_UNAVAILABLE,
                });
                send_response(&mut client_conn, &response).await;
                continue;
            }
        };
        log::info!(
            "{} -> {}: {}",
            client_ip,
            upstream_ip,
            request::format_request_line(&request)
        );

        if state.max_requests_per_minute > 0 {
            {
                let mut rate_limit_map = state.rate_limit_map.clone().lock_owned().await;
                let cnt = rate_limit_map.entry(client_ip.to_string()).or_insert(0);
                *cnt += 1;

                if *cnt > state.max_requests_per_minute.try_into().unwrap() {
                    let response = response::make_http_error(http::StatusCode::TOO_MANY_REQUESTS);
                    if let Err(error) = response::write_to_stream(&response, &mut client_conn).await
                    {
                        log::error!("failed to send response to client: {:?}", error);
                    }
                    continue;
                }
            }
        }

        // Add X-Forwarded-For header so that the upstream server knows the client's IP address.
        // (We're the ones connecting directly to the upstream server, so without this header, the
        // upstream server will only know our IP, not the client's.)
        request::extend_header_value(&mut request, "x-forwarded-for", &client_ip);

        // Forward the request to the server
        if let Err(error) = request::write_to_stream(&request, &mut upstream_conn).await {
            log::error!(
                "Failed to send request to upstream {}: {}",
                upstream_ip,
                error
            );
            let response = response::make_http_error(http::StatusCode::BAD_GATEWAY);
            send_response(&mut client_conn, &response).await;
            return;
        }
        log::debug!("Forwarded request to server");

        // Read the server's response
        let response = match response::read_from_stream(&mut upstream_conn, request.method()).await
        {
            Ok(response) => response,
            Err(error) => {
                log::error!("Error reading response from server: {:?}", error);
                let response = response::make_http_error(http::StatusCode::BAD_GATEWAY);
                send_response(&mut client_conn, &response).await;
                return;
            }
        };
        // Forward the response to the client
        send_response(&mut client_conn, &response).await;
        log::debug!("Forwarded response to client");
    }
}
