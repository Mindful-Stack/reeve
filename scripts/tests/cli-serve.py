"""A loopback stand-in for the server's /cli/ routes, for scripts/tests/cli-unix.sh and cli-windows.sh.

Serves <root> at http://127.0.0.1:<port>/, with the server's content types (text/plain for VERSION, SHA256SUMS and the
install scripts, application/octet-stream otherwise), and writes the port to <port-file> once it listens. Offers the
build on every response, as the server does: X-Reeve-Cli carries the first line of <root>/cli/VERSION when that is a
CLI version (40 lowercase hex), so `reeve update`'s "the build changed while updating" check sees one consistent build.
Appends one line per request to <log>: the method, the path, and "auth" when it carried an Authorization or Cookie
header, else "-". When <root>/redirects exists, a line "<path> <location>" makes a request for that exact path answer
302 to <location>. VERSION and redirects are read per request, so a test can write them after startup.

Standard library only (Python 3.7+). Usage: cli-serve.py <root> <port-file> <log>
"""
import functools
import http.server
import pathlib
import re
import sys
import threading

root, port_file, log = (pathlib.Path(a) for a in sys.argv[1:4])
TEXT = ("VERSION", "SHA256SUMS", "install.sh", "install.ps1")
BUILD = re.compile(r"[0-9a-f]{40}")
lock = threading.Lock()


def offered():
    """The build <root>/cli/VERSION names, or None."""
    try:
        first = (root / "cli" / "VERSION").read_text().splitlines()[0].strip()
    except (OSError, IndexError):
        return None
    return first if BUILD.fullmatch(first) else None


class Handler(http.server.SimpleHTTPRequestHandler):
    def handle_one(self, send):
        sent = "Authorization" in self.headers or "Cookie" in self.headers
        # newline="\n": on Windows text mode would write \r\n, and the checks grep for lines ending in "auth".
        with lock, log.open("a", newline="\n") as f:
            f.write(f"{self.command} {self.path} {'auth' if sent else '-'}\n")
        table = root / "redirects"
        for line in table.read_text().splitlines() if table.is_file() else []:
            source, _, location = line.partition(" ")
            if source == self.path:
                self.send_response(302)
                self.send_header("Location", location)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
        send()

    def end_headers(self):
        build = offered()
        if build:
            self.send_header("X-Reeve-Cli", build)
        super().end_headers()

    def do_GET(self):
        self.handle_one(super().do_GET)

    def do_HEAD(self):
        self.handle_one(super().do_HEAD)

    def guess_type(self, path):
        return "text/plain; charset=utf-8" if str(path).endswith(TEXT) else "application/octet-stream"

    def log_message(self, format, *args):
        sys.stderr.write("cli-serve: " + format % args + "\n")


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Handler, directory=str(root)))
partial = port_file.with_suffix(".partial")
partial.write_text(str(server.server_address[1]))
partial.replace(port_file)
server.serve_forever()
