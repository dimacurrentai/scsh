//! Command-scoped SSH network access for image-build downloads. Remote execution and
//! image pulls remain the runtime's job. Lifecycle verification: SSH-BUILD-PROXY.md.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, OnceLock};
use std::time::{Duration, Instant};

/// Immutable CLI selection, read only by image builds (including workflow builds).
static DESTINATION: OnceLock<Destination> = OnceLock::new();
/// Startup limit for network inspection and each SSH connection.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
/// The reviewed helper is embedded, never installed or written to the remote host.
const REMOTE_PROXY: &str = include_str!("build_proxy.py");

/// A validated SSH destination; SSH itself resolves aliases, users, keys and ports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination(String);

impl Destination {
  pub fn parse(value: &str) -> Result<Self, String> {
    if value.is_empty()
      || value.starts_with('-')
      || value.split('@').any(str::is_empty)
      || value.matches('@').count() > 1
      || !value.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.@:[]".contains(&b))
    {
      return Err("--ssh-proxy needs an SSH alias or username@host (configure ports and keys in ~/.ssh/config)".into());
    }
    Ok(Self(value.to_string()))
  }
}

/// Set once at CLI entry; recorder subprocesses already receive explicit build args.
pub fn configure(destination: Option<Destination>) {
  if let Some(destination) = destination {
    let _ = DESTINATION.set(destination);
  }
}

pub fn is_configured() -> bool {
  DESTINATION.get().is_some()
}

/// A process owned by this invocation, also covered by the existing signal cleanup.
struct OwnedChild {
  /// SSH process; stdin keeps the remote helper alive until this owner drops.
  child: Child,
}

impl OwnedChild {
  fn spawn(command: &mut Command) -> Result<Self, String> {
    crate::ui::signals::isolate_child(command);
    let child = command.spawn().map_err(|e| format!("could not start SSH build proxy: {e}; install ssh first"))?;
    crate::ui::signals::register_child(child.id());
    Ok(Self { child })
  }
}

impl Drop for OwnedChild {
  fn drop(&mut self) {
    drop(self.child.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
      if matches!(self.child.try_wait(), Ok(Some(_))) {
        if crate::ui::signals::orphaned_child_group(self.child.id()) {
          crate::ui::signals::signal_child_group(self.child.id(), "KILL");
        }
        crate::ui::signals::unregister_child(self.child.id());
        return;
      }
      std::thread::sleep(Duration::from_millis(10));
    }
    crate::ui::signals::signal_child_group(self.child.id(), "KILL");
    let _ = self.child.kill();
    let _ = self.child.wait();
    crate::ui::signals::unregister_child(self.child.id());
  }
}

/// Owns both SSH connections for exactly one image build; all exits tear them down.
pub struct BuildProxy {
  /// Stop forwarding before stopping the remote HTTP server.
  _tunnel: OwnedChild,
  /// Remote loopback-only Python server, terminated by SSH stdin EOF.
  _remote: OwnedChild,
  /// Container-reachable local bridge address; never a public/LAN listener.
  endpoint: SocketAddr,
}

impl BuildProxy {
  pub fn for_build(runtime: &str) -> Result<Option<Self>, String> {
    DESTINATION.get().map(|host| Self::start(runtime, host)).transpose()
  }

  fn start(runtime: &str, host: &Destination) -> Result<Self, String> {
    let gateway = bridge_gateway(runtime)?;
    let listener = TcpListener::bind(SocketAddr::new(gateway, 0)).map_err(|e| {
      format!("cannot bind SSH build proxy to local bridge {gateway}: {e}; use a local bridge runtime (Apple Containers or rootful Linux Docker/Podman)")
    })?;
    let endpoint = listener.local_addr().map_err(|e| e.to_string())?;
    let remote_command = format!("python3 -u -c {}", crate::runtime::shell_quote(REMOTE_PROXY));
    let mut remote = OwnedChild::spawn(
      ssh().arg(&host.0).arg(remote_command).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()),
    )?;
    let stdout = remote.child.stdout.take().expect("piped SSH stdout");
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
      let mut line = String::new();
      let result = BufReader::new(stdout.take(4096)).read_line(&mut line).map(|_| line);
      let _ = send.send(result);
    });
    let line = receive
      .recv_timeout(STARTUP_TIMEOUT)
      .map_err(|_| {
        format!("SSH proxy on {} did not become ready within 30s; check `ssh {} python3 --version`", host.0, host.0)
      })?
      .map_err(|e| e.to_string())?;
    let port = ready_port(&line)?;
    // Reserve an ephemeral port until SSH is ready to bind it. A competing bind is a
    // startup error, never permission to use somebody else's listener.
    drop(listener);
    let forward = format!("{endpoint}:127.0.0.1:{port}");
    let mut tunnel = OwnedChild::spawn(
      ssh()
        .args(["-N", "-L", &forward])
        .arg(&host.0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit()),
    )?;
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
      if let Some(status) = tunnel.child.try_wait().map_err(|e| e.to_string())? {
        return Err(format!(
          "SSH build proxy forwarding exited ({status}); check SSH AllowTcpForwarding and local bridge access"
        ));
      }
      if forwarding_ready(endpoint, port) {
        break;
      }
      if Instant::now() >= deadline {
        return Err("SSH build proxy forwarding timed out after 30s; check SSH AllowTcpForwarding".into());
      }
      std::thread::sleep(Duration::from_millis(20));
    }
    Ok(Self { _tunnel: tunnel, _remote: remote, endpoint })
  }

  /// Predefined proxy build args are excluded from Dockerfile content and fingerprints.
  /// Lowercase aliases cover tools such as curl; explicit bypasses override ambient NO_PROXY.
  pub fn build_args(&self) -> Vec<String> {
    proxy_args(self.endpoint)
  }
}

fn ssh() -> Command {
  let mut command = Command::new("ssh");
  command.args([
    "-T",
    "-S",
    "none",
    "-o",
    "ForkAfterAuthentication=no",
    "-o",
    "BatchMode=yes",
    "-o",
    "ConnectTimeout=15",
    "-o",
    "ConnectionAttempts=1",
    "-o",
    "ExitOnForwardFailure=yes",
    "-o",
    "ServerAliveInterval=10",
    "-o",
    "ServerAliveCountMax=2",
  ]);
  command
}

/// Verify the forwarding reaches this invocation's remote server, rather than merely
/// accepting a local TCP connection (which would miss disabled SSH forwarding).
fn forwarding_ready(endpoint: SocketAddr, remote_port: u16) -> bool {
  let Ok(mut socket) = TcpStream::connect_timeout(&endpoint, Duration::from_millis(100)) else {
    return false;
  };
  let timeout = Some(Duration::from_secs(1));
  if socket.set_read_timeout(timeout).is_err() || socket.set_write_timeout(timeout).is_err() {
    return false;
  }
  let request = format!("HEAD /.scsh-ready/{remote_port} HTTP/1.0\r\n\r\n");
  if socket.write_all(request.as_bytes()).is_err() {
    return false;
  }
  let mut response = String::new();
  if socket.take(4096).read_to_string(&mut response).is_err() {
    return false;
  }
  response.starts_with("HTTP/1.0 200 ")
    && response.lines().any(|line| line.trim() == format!("X-Scsh-Proxy-Port: {remote_port}"))
}

fn ready_port(line: &str) -> Result<u16, String> {
  line.trim().strip_prefix("SCSH_PROXY_READY ").and_then(|s| s.parse::<u16>().ok()).filter(|&p| p > 0).ok_or_else(
    || "SSH proxy did not report a valid port; check noninteractive SSH access and Python 3 on the destination".into(),
  )
}

fn proxy_args(endpoint: SocketAddr) -> Vec<String> {
  let url = format!("http://{endpoint}");
  ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy", "NO_PROXY", "no_proxy"]
    .into_iter()
    .flat_map(|name| {
      [
        "--build-arg".to_string(),
        format!("{name}={}", if name.eq_ignore_ascii_case("no_proxy") { "localhost,127.0.0.1,::1" } else { &url }),
      ]
    })
    .collect()
}

/// Inspect only the runtime's own bridge: never expose a temporary proxy on the LAN.
fn bridge_gateway(runtime: &str) -> Result<IpAddr, String> {
  let network = match runtime {
    "container" => "default",
    "docker" => "bridge",
    "podman" => "podman",
    _ => return Err(format!("SSH build proxy does not support runtime {runtime}")),
  };
  let out =
    crate::runtime::command_output(Command::new(runtime).args(["network", "inspect", network]), STARTUP_TIMEOUT)?;
  if !out.status.success() {
    return Err(format!(
      "cannot inspect {runtime} network {network}: {}; start the local container runtime first",
      String::from_utf8_lossy(&out.stderr).trim()
    ));
  }
  gateway_from_inspect(runtime, &String::from_utf8_lossy(&out.stdout))
}

fn gateway_from_inspect(runtime: &str, text: &str) -> Result<IpAddr, String> {
  use crate::json::Value;
  fn field<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    if let Value::Object(fields) = v {
      fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    } else {
      None
    }
  }
  fn first(v: &Value) -> Option<&Value> {
    if let Value::Array(values) = v {
      values.first()
    } else {
      None
    }
  }
  let value = crate::json::parse(text)?;
  let gateway = first(&value).and_then(|v| match runtime {
    "container" => field(field(v, "status")?, "ipv4Gateway"),
    "docker" => field(first(field(field(v, "IPAM")?, "Config")?)?, "Gateway"),
    "podman" => field(first(field(v, "subnets")?)?, "gateway"),
    _ => None,
  });
  if let Some(Value::String(ip)) = gateway {
    if let Ok(ip) = ip.parse::<IpAddr>() {
      if !ip.is_unspecified() && !ip.is_multicast() {
        return Ok(ip);
      }
    }
  }
  Err(format!(
    "cannot find a bridge gateway for {runtime}; SSH build proxy needs Apple Containers or rootful Linux Docker/Podman"
  ))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn accepts_ssh_aliases_users_and_ipv6_but_not_options_or_shell_text() {
    for host in ["build-proxy", "alice@linux.example.com", "alice@2001:db8::1", "[::1]"] {
      assert!(Destination::parse(host).is_ok(), "{host}");
    }
    for host in ["", "-oProxyCommand=bad", "a b", "a;exit", "a\nb", "@host", "user@", "a@b@c"] {
      assert!(Destination::parse(host).is_err(), "{host}");
    }
  }

  #[test]
  fn owned_children_are_reaped_on_success_and_early_failure() {
    for fail in [false, true] {
      let mut child = OwnedChild::spawn(
        Command::new("sh").args(["-c", "echo ready; read line"]).stdin(Stdio::piped()).stdout(Stdio::piped()),
      )
      .unwrap();
      let pid = child.child.id();
      let stdout = child.child.stdout.take().unwrap();
      let mut ready = String::new();
      BufReader::new(stdout).read_line(&mut ready).unwrap();
      assert_eq!(ready.trim(), "ready");
      let finish = || -> Result<(), ()> {
        let _owner = child;
        if fail {
          return Err(());
        }
        Ok(())
      };
      assert_eq!(finish().is_err(), fail);
      let status = Command::new("kill").args(["-0", &pid.to_string()]).output().unwrap();
      assert!(!status.status.success(), "owned child {pid} survived Drop");
    }
  }

  #[test]
  fn accepts_only_a_ready_port() {
    assert_eq!(ready_port("SCSH_PROXY_READY 12345\n").unwrap(), 12345);
    for text in ["", "12345", "SCSH_PROXY_READY 0", "SCSH_PROXY_READY 65536"] {
      assert!(ready_port(text).is_err());
    }
  }

  #[test]
  fn gateways_follow_each_runtime_schema() {
    for (runtime, text, expected) in [
      ("container", r#"[{"status":{"ipv4Gateway":"192.168.64.1"}}]"#, "192.168.64.1"),
      ("docker", r#"[{"IPAM":{"Config":[{"Gateway":"172.17.0.1"}]}}]"#, "172.17.0.1"),
      ("podman", r#"[{"subnets":[{"gateway":"10.88.0.1"}]}]"#, "10.88.0.1"),
    ] {
      assert_eq!(gateway_from_inspect(runtime, text).unwrap().to_string(), expected);
    }
    for text in ["[]", "{}", r#"[{"status":{"ipv4Gateway":"0.0.0.0"}}]"#] {
      assert!(gateway_from_inspect("container", text).is_err());
    }
  }

  #[test]
  fn forwarding_readiness_rejects_an_unrelated_or_broken_listener() {
    for (response, expected) in [
      ("HTTP/1.0 200 OK\r\nX-Scsh-Proxy-Port: 12345\r\n\r\n", true),
      ("HTTP/1.0 200 OK\r\nX-Scsh-Proxy-Port: 54321\r\n\r\n", false),
      ("HTTP/1.0 502 Bad Gateway\r\n\r\n", false),
    ] {
      let listener = TcpListener::bind("127.0.0.1:0").unwrap();
      let endpoint = listener.local_addr().unwrap();
      let server = std::thread::spawn(move || {
        let (mut client, _) = listener.accept().unwrap();
        client.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut request = [0; 256];
        let _ = client.read(&mut request).unwrap();
        client.write_all(response.as_bytes()).unwrap();
      });
      assert_eq!(forwarding_ready(endpoint, 12345), expected);
      server.join().unwrap();
    }
  }

  #[test]
  fn proxy_args_are_only_predefined_build_arguments() {
    let args = proxy_args("127.0.0.1:12345".parse().unwrap());
    assert_eq!(args.len(), 12);
    assert_eq!(&args[..2], ["--build-arg", "HTTP_PROXY=http://127.0.0.1:12345"]);
    assert!(args.contains(&"https_proxy=http://127.0.0.1:12345".into()));
    assert!(args.contains(&"NO_PROXY=localhost,127.0.0.1,::1".into()));
    assert!(!args.iter().any(|s| s.contains("--env") || s.contains("--label")));
  }
}
