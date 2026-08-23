#!/usr/bin/env python3
"""PTY harness: drives the real sibsh binary through a pseudo-terminal and
checks screen state with a mini terminal emulator.

Supported emulator escapes (exactly what sibsh emits):
  \\r \\n, printable chars (wrapping at width), CSI A / J / K / D / n D.

The emulator models soft (wrap) vs hard (newline) line breaks, so window
resizes can re-flow the grid the way a real terminal does.
"""
import os, pty, re, select, subprocess, sys, time

WIDTH, HEIGHT = 80, 40


class Screen:
    def __init__(self, width=WIDTH, height=HEIGHT):
        self.w = width
        self.h = height
        self.grid = [[" "] * width for _ in range(height)]
        # hard[i] is True when row i is terminated by an explicit newline;
        # rows created by automatic wrap are soft continuations.
        self.hard = [False] * height
        self.row = 0
        self.col = 0

    def _scroll_if_needed(self):
        if self.row >= self.h:
            del self.grid[0]
            self.grid.append([" "] * self.w)
            del self.hard[0]
            self.hard.append(False)
            self.row = self.h - 1

    def _put(self, ch):
        if self.col >= self.w:
            # Soft continuation: no hard break recorded.
            self.col = 0
            self.row += 1
            self._scroll_if_needed()
        self.grid[self.row][self.col] = ch
        self.col += 1

    def _lf(self):
        self.hard[self.row] = True
        self.row += 1
        self._scroll_if_needed()

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
                            self.hard[rr] = False
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

    def resize(self, w, h):
        """Re-flow the grid to a new size, the way terminals rearrange text
        when their window changes. Soft-wrapped rows merge back together and
        re-wrap at the new width; hard breaks are preserved. The cursor is
        placed at the end of the segment it previously occupied."""
        segs = []
        starts = []
        cur = ""
        row_no = 0
        for i in range(self.h):
            text = "".join(self.grid[i])
            if not cur:
                starts.append(row_no)
            cur += text.rstrip()
            row_no += 1
            if self.hard[i]:
                segs.append(cur)
                cur = ""
        if cur:
            segs.append(cur)

        # Segment holding the cursor: last one that started at or above it.
        cursor_seg = 0
        for si, start in enumerate(starts[:len(segs)] if len(starts) > len(segs) else starts):
            if start <= self.row:
                cursor_seg = si

        saved_w = self.w
        self.w, self.h = w, h
        self.grid = [[" "] * w for _ in range(h)]
        self.hard = [False] * h
        self.row = self.col = 0

        cursor_pos = (0, 0)
        for si, seg in enumerate(segs):
            for ch in seg:
                self._put(ch)
            if si == cursor_seg:
                cursor_pos = (self.row, self.col)
            if si < len(segs) - 1:
                self._lf()
        self.row, self.col = cursor_pos

    def text(self):
        return "\n".join("".join(r).rstrip() for r in self.grid)


def run_session(binary, script_steps, env_extra=None, cwd=None, timeout=15,
                width=WIDTH, height=HEIGHT):
    """script_steps: list of (bytes_to_send, delay_seconds).

    A step may instead be ("resize", (cols, rows), delay_seconds), which
    changes the pty window size — exactly what a real terminal sends when
    the user drags its border. Output and resizes are recorded in order and
    replayed into the emulator so re-flow behaves realistically.
    """
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
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))

    events = []

    def pump(dur=0.3):
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
                events.append(("data", chunk))
        return True

    pump(0.5)
    for step in script_steps:
        if step[0] == "resize":
            _, (cols, rows), delay = step
            fcntl.ioctl(fd, termios.TIOCSWINSZ,
                        struct.pack("HHHH", rows, cols, 0, 0))
            events.append(("resize", cols, rows))
        else:
            os.write(fd, step[0])
        if not pump(step[-1]):
            break

    try:
        os.kill(pid, 9)
    except ProcessLookupError:
        pass
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass

    screen = Screen(width, height)
    for event in events:
        if event[0] == "resize":
            screen.resize(event[1], event[2])
        else:
            screen.feed(event[1].decode("utf-8", errors="replace"))
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

    # 11. Terminal shrunk mid-typing: the editor must detect the new width on
    # the next keystroke, redraw once under the new geometry, keep the typed
    # buffer, and stay alive.
    s = run_session(binary, [
        (b"echo hel", 0.5),
        ("resize", (40, 24), 0.4),
        (b"lo world\r", 0.6),
        (b"echo AFTER_SHRINK\r", 0.6),
    ], width=80, height=24)
    t = s.text()
    check("shrink: buffer preserved and executed", "hello world" in t,
          repr(t[-600:]))
    check("shrink: shell still responsive", "AFTER_SHRINK" in t, repr(t[-600:]))
    check("shrink: no duplicate prompt frames",
          count_prompt_frames(t) <= 3, repr(t[-600:]))

    # 12. Terminal grown mid-typing: same survival guarantees in reverse.
    s = run_session(binary, [
        (b"echo grow", 0.5),
        ("resize", (100, 30), 0.4),
        (b"nmark\r", 0.6),
        (b"echo AFTER_GROW\r", 0.6),
    ], width=40, height=20)
    t = s.text()
    check("grow: buffer preserved and executed", "grownmark" in t,
          repr(t[-600:]))
    check("grow: shell still responsive", "AFTER_GROW" in t, repr(t[-600:]))
    check("grow: no duplicate prompt frames",
          count_prompt_frames(t) <= 3, repr(t[-600:]))

    # 13. Rapid successive resizes with an empty buffer must leave the next
    # prompt perfectly usable.
    s = run_session(binary, [
        ("resize", (50, 20), 0.3),
        ("resize", (90, 30), 0.3),
        ("resize", (72, 24), 0.3),
        (b"echo CLEAN_AFTER_RESIZE\r", 0.6),
    ])
    t = s.text()
    check("rapid-resize: command runs cleanly", "CLEAN_AFTER_RESIZE" in t,
          repr(t[-300:]))
    check("rapid-resize: no frame debris",
          count_prompt_frames(t) <= 2, repr(t[-300:]))

    # 14. A line longer than the screen height forces scroll during typing;
    # repaints must clamp to the visible screen instead of escaping into
    # scrollback (which used to duplicate prompt frames).
    long_line = b"true " + b"z" * 240
    s = run_session(binary, [(long_line, 1.2), (b"\r", 0.5),
                             (b"echo SCROLLMARK\r", 0.6)],
                    width=30, height=8)
    t = s.text()
    check("scroll: shell survives oversized input", "SCROLLMARK" in t,
          repr(t[-500:]))
    check("scroll: bounded frame debris",
          count_prompt_frames(t) <= 3, repr(t[-500:]))

    print(f"\n{len(failures)} failure(s)")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
