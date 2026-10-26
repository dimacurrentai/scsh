"""Temporary HTTP/CONNECT proxy, owned by the SSH channel's stdin lifetime.

Protocol behavior is exercised by scripts/test_build_proxy.py; SSH/container lifecycle is
verified by SSH-BUILD-PROXY.md. No dependencies or remote files are installed.
"""
import http.server
import os
import select
import socket
import threading
import urllib.parse


class Proxy(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def relay(self, upstream):
        while True:
            ready, _, _ = select.select([self.connection, upstream], [], [], 60)
            if not ready:
                return
            for source in ready:
                data = source.recv(65536)
                if not data:
                    return
                destination = upstream if source is self.connection else self.connection
                destination.sendall(data)

    def do_CONNECT(self):
        try:
            host, port = self.path.rsplit(":", 1)
            try:
                upstream = socket.create_connection((host.strip("[]"), int(port)), 30)
            except OSError:
                self.send_error(502, "Proxy could not connect to destination")
                return
            with upstream:
                self.send_response(200, "Connection Established")
                self.end_headers()
                self.relay(upstream)
        except (OSError, ValueError):
            self.close_connection = True

    def do_GET(self):
        try:
            url = urllib.parse.urlsplit(self.path)
            if url.scheme != "http" or not url.hostname or url.username or url.password:
                self.send_error(400, "Expected an absolute HTTP URL")
                return
            try:
                upstream = socket.create_connection((url.hostname, url.port or 80), 30)
            except OSError:
                self.send_error(502, "Proxy could not connect to destination")
                return
            with upstream:
                path = urllib.parse.urlunsplit(("", "", url.path or "/", url.query, ""))
                excluded = {"connection", "proxy-connection", "proxy-authorization", "host"}
                headers = "".join(f"{k}: {v}\r\n" for k, v in self.headers.items() if k.lower() not in excluded)
                request = f"{self.command} {path} HTTP/1.0\r\nHost: {url.netloc}\r\n{headers}Connection: close\r\n\r\n"
                upstream.sendall(request.encode("iso-8859-1"))
                self.relay(upstream)
        except (OSError, ValueError, UnicodeError):
            self.close_connection = True

    def do_HEAD(self):
        if self.path == f"/.scsh-ready/{self.server.server_port}":
            self.send_response(200)
            self.send_header("X-Scsh-Proxy-Port", str(self.server.server_port))
            self.end_headers()
        else:
            self.do_GET()


class Server(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def handle_error(self, request, client_address):
        pass


def main():
    # SSH disconnect, normal shutdown, and local SIGINT all close stdin. Exit even
    # if a client has stalled, without leaving a remote daemon behind.
    def watch_owner():
        import sys
        sys.stdin.buffer.read()
        os._exit(0)

    server = Server(("127.0.0.1", 0), Proxy)
    threading.Thread(target=watch_owner, daemon=True).start()
    print(f"SCSH_PROXY_READY {server.server_port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
