#!/usr/bin/env python3
"""Minimal Telnet test host for QShell.

Usage:
    python telnet_test_host.py
    python telnet_test_host.py --host 0.0.0.0 --port 2323

Then create a QShell host:
    Protocol: Telnet
    Host:     127.0.0.1
    Port:     2323

It handles the common Telnet negotiations (ECHO, SGA, NAWS, TTYPE),
prints a prompt, echoes input, and answers `dir`, `help`, `ver`, `cls`,
and `exit`.  No login is required.
"""

from __future__ import annotations

import argparse
import datetime as _dt
import socket
import socketserver
import threading
import time

IAC = 255
DONT = 254
DO = 253
WONT = 252
WILL = 251
SB = 250
SE = 240
OPT_ECHO = 1
OPT_SGA = 3
OPT_TTYPE = 24
OPT_NAWS = 31

PROMPT = r"C:\TEST> "
BANNER = (
    "\r\n"
    "QShell Python Telnet Test Host\r\n"
    "--------------------------------\r\n"
    "Type \"dir\" and press Enter.\r\n"
    "Type \"help\" for commands.\r\n"
    "\r\n"
)


def iac(*values: int) -> bytes:
    return bytes(values)


def quoted(data: bytes) -> bytes:
    return data.replace(bytes([IAC]), bytes([IAC, IAC]))


class TelnetState:
    def __init__(self) -> None:
        self.state = "data"
        self.option = 0
        self.sub: list[int] = []
        self.cols = 80
        self.rows = 24
        self.terminal = "unknown"
        self.do_echo = False
        self.will_echo = False
        self.do_sga = False
        self.will_sga = False
        self.do_naws = False
        self.will_naws = False
        self.do_ttype = False
        self.will_ttype = False

    def negotiate(self, command: int, option: int, send: callable) -> None:
        if command == WILL:
            if option == OPT_ECHO:
                if not self.will_echo:
                    self.will_echo = True
                    send(iac(IAC, DO, OPT_ECHO))
            elif option == OPT_SGA:
                if not self.will_sga:
                    self.will_sga = True
                    send(iac(IAC, DO, OPT_SGA))
            elif option == OPT_NAWS:
                if not self.will_naws:
                    self.will_naws = True
                    send(iac(IAC, DO, OPT_NAWS))
            elif option == OPT_TTYPE:
                if not self.will_ttype:
                    self.will_ttype = True
                    send(iac(IAC, DO, OPT_TTYPE))
                    send(iac(IAC, SB, OPT_TTYPE, 1, IAC, SE))
            else:
                send(iac(IAC, DONT, option))
        elif command == DO:
            if option == OPT_ECHO:
                if not self.do_echo:
                    self.do_echo = True
                    send(iac(IAC, WILL, OPT_ECHO))
            elif option == OPT_SGA:
                if not self.do_sga:
                    self.do_sga = True
                    send(iac(IAC, WILL, OPT_SGA))
            elif option == OPT_NAWS:
                if not self.do_naws:
                    self.do_naws = True
                    send(iac(IAC, WILL, OPT_NAWS))
            elif option == OPT_TTYPE:
                if not self.do_ttype:
                    self.do_ttype = True
                    send(iac(IAC, WILL, OPT_TTYPE))
            else:
                send(iac(IAC, WONT, option))
        elif command == WONT:
            if option == OPT_ECHO:
                self.will_echo = False
            elif option == OPT_SGA:
                self.will_sga = False
            elif option == OPT_NAWS:
                self.will_naws = False
            elif option == OPT_TTYPE:
                self.will_ttype = False
        elif command == DONT:
            if option == OPT_ECHO:
                self.do_echo = False
            elif option == OPT_SGA:
                self.do_sga = False
            elif option == OPT_NAWS:
                self.do_naws = False
            elif option == OPT_TTYPE:
                self.do_ttype = False

    def handle_subneg(self, send: callable) -> None:
        if len(self.sub) >= 2 and self.sub[0] == OPT_TTYPE and self.sub[1] == 1:
            send(
                iac(IAC, SB, OPT_TTYPE, 0)
                + quoted(b"xterm-256color")
                + iac(IAC, SE)
            )
            self.terminal = "xterm-256color"
        elif len(self.sub) >= 5 and self.sub[0] == OPT_NAWS:
            self.cols = (self.sub[1] << 8) | self.sub[2]
            self.rows = (self.sub[3] << 8) | self.sub[4]

    def feed(self, data: bytes, send: callable) -> bytes:
        """Remove Telnet commands and return application data."""
        out = bytearray()
        for byte in data:
            if self.state == "data":
                if byte == IAC:
                    self.state = "iac"
                else:
                    out.append(byte)
            elif self.state == "iac":
                if byte == IAC:
                    out.append(IAC)
                    self.state = "data"
                elif byte in (WILL, WONT, DO, DONT):
                    self.option = byte
                    self.state = "option"
                elif byte == SB:
                    self.sub = []
                    self.state = "sub"
                else:
                    self.state = "data"
            elif self.state == "option":
                self.negotiate(self.option, byte, send)
                self.state = "data"
            elif self.state == "sub":
                if byte == IAC:
                    self.state = "sub_iac"
                else:
                    self.sub.append(byte)
            elif self.state == "sub_iac":
                if byte == SE:
                    self.handle_subneg(send)
                    self.state = "data"
                elif byte == IAC:
                    self.sub.append(IAC)
                    self.state = "sub"
                else:
                    self.state = "data"
        return bytes(out)


class TelnetHandler(socketserver.BaseRequestHandler):
    def setup(self) -> None:
        self.request.settimeout(0.5)
        self.state = TelnetState()
        self.line = ""
        self.skip_lf = False
        self.alive = True
        self.lock = threading.Lock()

    def send(self, data: bytes) -> None:
        with self.lock:
            if not self.alive:
                return
            try:
                self.request.sendall(data)
            except OSError:
                self.alive = False

    def write_text(self, text: str) -> None:
        self.send(text.encode("utf-8", errors="replace"))

    def prompt(self) -> None:
        self.write_text(PROMPT)

    def dir_listing(self) -> None:
        today = _dt.datetime.now().strftime("%Y-%m-%d  %H:%M")
        self.write_text(
            "\r\n"
            " Volume in drive C has no label.\r\n"
            " Volume Serial Number is 51F7-7E2A\r\n"
            "\r\n"
            " Directory of C:\\TEST\r\n"
            "\r\n"
            f"{today}    <DIR>          .\r\n"
            f"{today}    <DIR>          ..\r\n"
            f"{today}               118  readme.txt\r\n"
            f"{today}               246  qshell-test.log\r\n"
            f"{today}    <DIR>          drivers\r\n"
            "               2 File(s)            364 bytes\r\n"
            "               3 Dir(s)   8,196,112,384 bytes free\r\n"
            "\r\n"
        )

    def process_line(self) -> None:
        line = self.line.strip()
        self.line = ""
        if not line:
            self.prompt()
            return

        command = line.lower()
        if command in ("dir", "ls"):
            self.dir_listing()
        elif command == "help":
            self.write_text(
                "\r\n"
                "Commands:\r\n"
                "  dir    list files\r\n"
                "  ver    show version\r\n"
                "  cls    clear screen\r\n"
                "  exit   disconnect\r\n"
                "\r\n"
            )
        elif command == "ver":
            self.write_text(
                "\r\nQShell Python Telnet Test Host v1.0\r\n"
                "Telnet service is running.\r\n\r\n"
            )
        elif command == "cls":
            self.write_text("\x1b[2J\x1b[H")
        elif command in ("exit", "quit", "logout"):
            self.write_text("\r\nGoodbye.\r\n")
            self.alive = False
            return
        else:
            self.write_text(f"\r\nUnknown command: {line}\r\n\r\n")
        self.prompt()

    def handle(self) -> None:
        peer = self.client_address
        print(f"[telnet] connected: {peer}", flush=True)
        try:
            # Ask the client for its terminal type and window size.
            self.send(
                iac(IAC, WILL, OPT_ECHO)
                + iac(IAC, WILL, OPT_SGA)
                + iac(IAC, DO, OPT_NAWS)
                + iac(IAC, DO, OPT_TTYPE)
            )
            self.write_text(BANNER)
            self.prompt()

            while self.alive:
                try:
                    data = self.request.recv(4096)
                except socket.timeout:
                    continue
                except OSError:
                    break
                if not data:
                    break

                clean = self.state.feed(data, self.send)
                for byte in clean:
                    if byte == 13:
                        self.write_text("\r\n")
                        self.skip_lf = True
                        self.process_line()
                        if not self.alive:
                            break
                    elif byte == 10:
                        # QShell sends CRLF for Enter; ignore the LF half.
                        if self.skip_lf:
                            self.skip_lf = False
                            continue
                        self.write_text("\r\n")
                        self.process_line()
                        if not self.alive:
                            break
                    elif byte in (8, 127):
                        self.skip_lf = False
                        if self.line:
                            self.line = self.line[:-1]
                            self.write_text("\b \b")
                    elif byte == 3:
                        self.skip_lf = False
                        self.line = ""
                        self.write_text("^C\r\n")
                        self.prompt()
                    elif 32 <= byte < 127:
                        self.skip_lf = False
                        self.line += chr(byte)
                        self.write_text(chr(byte))
                    elif byte >= 128:
                        # Keep bytes as latin1 so UTF-8 sequences are not lost
                        # before the line is decoded by the test host.
                        self.skip_lf = False
                        self.line += bytes([byte]).decode("latin1")
                        self.write_text("")
        finally:
            self.alive = False
            print(f"[telnet] disconnected: {peer}", flush=True)
            try:
                self.request.close()
            except OSError:
                pass


class ThreadingTelnetServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


def main() -> None:
    parser = argparse.ArgumentParser(description="QShell Telnet test host")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=2323)
    args = parser.parse_args()

    with ThreadingTelnetServer((args.host, args.port), TelnetHandler) as server:
        print(f"QShell Telnet test host listening on {args.host}:{args.port}", flush=True)
        print(f"Create a Telnet host in QShell: 127.0.0.1 : {args.port}", flush=True)
        print("Press Ctrl+C to stop.", flush=True)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            print("\nStopping.", flush=True)


if __name__ == "__main__":
    main()
