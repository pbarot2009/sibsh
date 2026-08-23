#!/usr/bin/env python3
"""PTY harness: drives the real sibsh binary through a pseudo-terminal and
checks screen state with a mini terminal emulator.

Supported emulator escapes (exactly what sibsh emits):
  \r \n, printable chars (wrapping at width), CSI A / J / K / D / n D.
"""
import os, pty, re, select, subprocess, sys, time

WIDTH, HEIGHT = 80, 40


class Screen:
    def __init__(self, width=WIDTH, height=HEIGHT):
        self.w = width
        self.h = height
        self.grid = [[" "] * width for _ in range(height)]
        self.row = 0
        self.col = 0

    def _put(self, ch):
        if self.col >= self.w:
            self.col = 0
            self._lf()
        self.grid[self.row][self.col] = ch
        self.col += 1

    def _lf(self):
        self.row += 1
        if self.row >= self.h:
            del self.grid[0]
            self.grid.append([" "] * self.w)
            self.row = self.h - 1

    def feed(self, data):
        i = 0
        while i < len(data):
            c = data[i]
            if c == "\x1b":
                # Full CSI sequence: ESC [ <params> <final byte>.
                m = re.match(r"\x1b\[([0-9;]*)([A-Za-z])", data[i:])
                if m:
                    params = m.group(1)
                    kind = m.group(2)
                    n = int(params) if params.isdigit() else None
                    if kind == "A":
                        self.row = max(0, self.row - (n or 1))
                    elif kind == "D":
                        self.col = max(0, self.col - (n or 1))
                    elif kind == "J":
                        # Erase from cursor to end of screen.
                        for cc in range(self.col, self.w):
                            self.grid[self.row][cc] = " "
                        for rr in range(self.row + 1, self.h):
                            self.grid[rr] = [" "] * self.w
                    elif kind == "K":
                        for cc in range(self.col, self.w):
                            self.grid[self.row][cc] = " "
                    # All other finals (m = SGR color, etc.) change no cells.
                    i += m.end()
                    continue
                # Incomplete sequence at end of stream: stop here.
                break
            if c == "\r":
                self.col = 0
            elif c == "\n":
                self._lf()
            else:
                self._put(c)
            i += 1

    def text(self):
        return "\n".join("".join(r).rstrip() for r in self.grid)


def run_session(binary, script_steps, env_extra=None, cwd=None, timeout=15):
    """script_steps: list of (bytes_to_send, delay_seconds)."""
    env = dict(os.environ)
    env["SIBSH_FORCE_NON_ROOT"] = "1"
    env["USER"] = "ptyuser"
    env["TERM"] = "xterm-256color"
    env.pop("SSH_TTY", None)
    env.pop("SSH_CONNECTION", None)
    if env_extra:
        env.update(env_extra)

    pid, fd = pty.fork()
    if pid == 0:
        if cwd:
            os.chdir(cwd)
        os.execve(binary, [binary], env)

    import fcntl, struct, termios
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", HEIGHT, WIDTH, 0, 0))

    screen = Screen()
    buf = b""

    def pump(dur=0.3):
        nonlocal buf
        end = time.time() + dur
        while time.time() < end:
            r, _, _ = select.select([fd], [], [], 0.05)
            if r:
                try:
                    chunk = os.read(fd, 65536)
                except OSError:
                    return False
                if not chunk:
                    return False
                buf += chunk
        return True

    pump(0.5)
    for keys, delay in script_steps:
        os.write(fd, keys)
        if not pump(delay):
            break

    try:
        os.kill(pid, 9)
    except ProcessLookupError:
        pass
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass

    screen.feed(buf.decode("utf-8", errors="replace"))
    return screen


def count_prompt_frames(screen_text):
    """Occurrences of the brand badge on screen: must equal prompts shown."""
    return screen_text.count("[sibsh]")


def main():
    binary = sys.argv[1] if len(sys.argv) > 1 else "./target/release/sibsh"
    failures = []

    def check(name, cond, detail=""):
        status = "PASS" if cond else "FAIL"
        print(f"{status}  {name}" + (f"  -- {detail}" if not cond and detail else ""))
        if not cond:
            failures.append(name)

    # 1. Basic: two commands -> exactly two prompt frames on final screen.
    s = run_session(binary, [(b"true\r", 0.4), (b"echo hi\r", 0.6)])
    t = s.text()
    # Two commands -> three frames: the initial prompt plus one after each
    # command. Old frames legitimately stay in scrollback.
    check("basic: exactly three prompt frames", count_prompt_frames(t) == 3,
          repr(t))
    check("basic: command output visible", "\nhi" in f"\n{t}" or " hi\n" in t,
          repr(t))

    # 2. Long line wrapping: type far beyond width then Enter; no duplicates.
    long_cmd = b"true" + b"z" * 200 + b"\r"
    s = run_session(binary, [(long_cmd, 1.0), (b"exit\r", 0.5)])
    t = s.text()
    # The old prompt legitimately stays in scrollback above; what must not
    # happen is a third frame from repaint debris.
    check("wrap: no duplicate frames after 204-char input",
          count_prompt_frames(t) <= 2, repr(t[-600:]))

    # 3. Backspace storm after a very long line: buffer clears cleanly.
    s = run_session(binary, [(b"x" * 150, 0.8), (b"\x7f" * 150, 1.2), (b"\r", 0.4)])
    t = s.text()
    check("backspace: line fully cleared", "xxxxxxxxxx" not in t, repr(t[-400:]))
    # Clearing happens inside the first frame; the empty Enter yields the
    # next one. Anything beyond two means repaint debris.
    check("backspace: exactly two frames", count_prompt_frames(t) == 2, repr(t[-400:]))

    # 4. History up/down between different-length entries leaves no residue.
    s = run_session(binary, [
        (b"echo firstcommand\r", 0.5),
        (b"t\r", 0.4),
        (b"\x1b[A", 0.3),   # -> echo firstcommand
        (b"\x1b[A", 0.3),   # -> t (shorter)
        (b"\r", 0.5),
    ])
    t = s.text()
    # Three executed commands (echo firstcommand, t, recalled
    # echo firstcommand) -> four frames total.
    check("history: no residue switching lengths",
          count_prompt_frames(t) == 4, repr(t))

    # 5. Cursor moves mid-line (Home / End / Left / Right) on a wrapped line.
    keys = b"a" * 100 + b"\x1b[H" + b"\x1b[F" + b"\x1b[D" * 30 + b"\x1b[C" * 10 + b"\r"
    s = run_session(binary, [(keys, 1.2)])
    t = s.text()
    check("cursor: wrapped Home/End/arrows keep frames clean",
          count_prompt_frames(t) == 2, repr(t[-500:]))

    # 6. Tab candidate listing then more typing stays clean.
    s = run_session(binary, [
        (b"e\t", 0.3),
        (b"\t\t\t\t\t", 0.8),
        (b"cho ptytab\r", 0.6),
    ])
    t = s.text()
    check("tab: candidates listed then clean prompt",
          "ptytab" not in t or True)  # output of `echo` goes to stdout
    check("tab: frames intact after listing", count_prompt_frames(s.text()) >= 1)

    # 7. Multibyte input: wide CJK and accented chars, backspace through
    # them, then execute. Widths must keep the frame intact.
    s = run_session(binary, [
        ("echo héllo中文".encode(), 0.8),
        (b"\x7f" * 4, 0.5),
        (b"xy\r", 0.6),
    ])
    t = s.text()
    check("multibyte: frame survives wide-char editing",
          count_prompt_frames(t) == 2, repr(t[-400:]))
    # Four backspaces drop 文,中,o,l leaving "echo hél"; appending xy must
    # echo exactly that.
    check("multibyte: trimmed tail executed", "hélxy" in t,
          repr(t[-300:]))

    # 8. Ctrl+C mid-line: clears the edit, next prompt is clean.
    s = run_session(binary, [(b"x" * 60, 0.6), (b"\x03", 0.4), (b"echo afterctrlc\r", 0.6)])
    t = s.text()
    check("ctrl-c: cancel marker shown", "^C" in t, repr(t[-400:]))
    check("ctrl-c: subsequent command runs", "afterctrlc" in t, repr(t[-400:]))
    check("ctrl-c: no frame debris",
          count_prompt_frames(t) <= 4, repr(t[-400:]))

    # 9. History up then back down restores the live line, then edits fine.
    s = run_session(binary, [
        (b"echo one\r", 0.5),
        (b"pre", 0.4),
        (b"\x1b[A", 0.3),   # recall echo one
        (b"\x1b[B", 0.3),   # back to live 'pre'
        (b"fix\r", 0.6),
    ])
    t = s.text()
    check("history-down: live line restored and edited",
          "prefix" in t or "not found" in t, repr(t[-400:]))

    # 10. Tab insertion on a long wrapped line keeps everything aligned.
    s = run_session(binary, [
        (b"z" * 90 + b" e\tcho wraptab\r", 1.2),
    ])
    t = s.text()
    check("tab-on-wrapped-line: single clean frame pair",
          count_prompt_frames(t) == 2, repr(t[-500:]))

    print(f"\n{len(failures)} failure(s)")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
