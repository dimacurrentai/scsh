"""Network-free coverage of the embedded SSH proxy, using only ephemeral loopback ports."""
import contextlib
import http.server
import importlib.util
import pathlib
import socket
import subprocess
import sys
import threading
import unittest

SOURCE = pathlib.Path(__file__).resolve().parents[1] / "src" / "build_proxy.py"
spec = importlib.util.spec_from_file_location("build_proxy", SOURCE)
proxy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proxy)


@contextlib.contextmanager
def serving(server):
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield server
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


def request(address, data):
    with socket.create_connection(address, 2) as client:
        client.settimeout(2)
        client.sendall(data)
        response = bytearray()
        while chunk := client.recv(65536):
            response.extend(chunk)
        return bytes(response)


class ProxyTests(unittest.TestCase):
    def test_http_and_head_forward_target_and_auth_without_proxy_headers(self):
        observed = []

        class Origin(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                observed.append((self.command, self.path, dict(self.headers)))
                self.send_response(200)
                self.send_header("Content-Length", "2")
                self.end_headers()
                if self.command != "HEAD":
                    self.wfile.write(b"OK")

            do_HEAD = do_GET

        with serving(http.server.ThreadingHTTPServer(("127.0.0.1", 0), Origin)) as origin:
            with serving(proxy.Server(("127.0.0.1", 0), proxy.Proxy)) as server:
                for method in ["GET", "HEAD"]:
                    url = f"http://127.0.0.1:{origin.server_port}/package?version=1"
                    message = (
                        f"{method} {url} HTTP/1.1\r\nHost: wrong.example\r\n"
                        "Authorization: test-origin-token\r\nProxy-Authorization: test-proxy-token\r\n\r\n"
                    )
                    response = request(server.server_address, message.encode())
                    self.assertIn(b"200 OK", response)
                    self.assertEqual(response.endswith(b"OK"), method == "GET")
        for method, path, headers in observed:
            self.assertIn(method, ["GET", "HEAD"])
            self.assertEqual(path, "/package?version=1")
            self.assertEqual(headers["Authorization"], "test-origin-token")
            self.assertNotIn("Proxy-Authorization", headers)
            self.assertNotEqual(headers["Host"], "wrong.example")

    def test_connect_is_bidirectional_binary_transport(self):
        import socketserver

        class Echo(socketserver.BaseRequestHandler):
            def handle(self):
                while data := self.request.recv(65536):
                    self.request.sendall(data)

        with serving(socketserver.ThreadingTCPServer(("127.0.0.1", 0), Echo)) as origin:
            with serving(proxy.Server(("127.0.0.1", 0), proxy.Proxy)) as server:
                with socket.create_connection(server.server_address, 2) as client:
                    client.settimeout(2)
                    port = origin.server_address[1]
                    client.sendall(f"CONNECT 127.0.0.1:{port} HTTP/1.0\r\n\r\n".encode())
                    headers = bytearray()
                    while not headers.endswith(b"\r\n\r\n"):
                        headers.extend(client.recv(1))
                    self.assertIn(b"200 Connection Established", headers)
                    payload = bytes(range(256)) * 100
                    client.sendall(payload)
                    received = bytearray()
                    while len(received) < len(payload):
                        received.extend(client.recv(65536))
                    self.assertEqual(received, payload)

    def test_invalid_http_targets_are_rejected(self):
        with serving(proxy.Server(("127.0.0.1", 0), proxy.Proxy)) as server:
            for target in ["/relative", "https://example.com/", "http://user:password@example.com/"]:
                response = request(server.server_address, f"GET {target} HTTP/1.0\r\n\r\n".encode())
                self.assertIn(b"400 Expected an absolute HTTP URL", response)

    def test_refused_destinations_report_a_proxy_error(self):
        # Stub only the external connection failure; the client's real loopback socket
        # is established before patching, so the HTTP response still crosses the wire.
        from unittest.mock import patch
        with serving(proxy.Server(("127.0.0.1", 0), proxy.Proxy)) as server:
            for target in ["GET http://unreachable.invalid/", "CONNECT unreachable.invalid:443"]:
                with socket.create_connection(server.server_address, 2) as client:
                    client.settimeout(2)
                    with patch.object(proxy.socket, "create_connection", side_effect=ConnectionRefusedError):
                        client.sendall(f"{target} HTTP/1.0\r\n\r\n".encode())
                        response = bytearray()
                        while data := client.recv(65536):
                            response.extend(data)
                    self.assertIn(b"502 Proxy could not connect to destination", response)

    def test_remote_helper_exits_when_ssh_stdin_closes(self):
        child = subprocess.Popen(
            [sys.executable, "-u", str(SOURCE)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE
        )
        try:
            # The helper is local and has no network dependency before readiness.
            import selectors
            with selectors.DefaultSelector() as selector:
                selector.register(child.stdout, selectors.EVENT_READ)
                self.assertTrue(selector.select(timeout=5), "helper readiness deadline")
            line = child.stdout.readline().decode().strip()
            self.assertTrue(line.startswith("SCSH_PROXY_READY "), line)
            port = int(line.split()[1])
            response = request(("127.0.0.1", port), f"HEAD /.scsh-ready/{port} HTTP/1.0\r\n\r\n".encode())
            self.assertIn(f"X-Scsh-Proxy-Port: {port}".encode(), response)
            child.stdin.close()
            child.wait(timeout=5)
            self.assertEqual(child.returncode, 0)
            with self.assertRaises(OSError):
                socket.create_connection(("127.0.0.1", port), 0.2)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=5)
            child.stdout.close()
            child.stderr.close()
            if not child.stdin.closed:
                child.stdin.close()


if __name__ == "__main__":
    unittest.main()
