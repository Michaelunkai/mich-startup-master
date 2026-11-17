#!/usr/bin/env python3
"""MCP-stdio driver for the multiagents orchestrator.
usage: team_ctl.py list | team_ctl.py call <tool> <json-args>"""
import json, subprocess, sys, threading, queue

CMD = ["cmd", "/c", "F:/backup/multiagents/orch-wrap.cmd"]

class Mcp:
    def __init__(self):
        self.p = subprocess.Popen(CMD, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE, text=True, encoding="utf-8",
                                  cwd="F:/study/Windows/Applications/Desktop/Utilities/System/Startup/Managers/mich-startup-master/design-ctl")
        self.q = queue.Queue()
        self.err = []
        threading.Thread(target=self._reader, daemon=True).start()
        threading.Thread(target=self._errpump, daemon=True).start()
        self.next_id = 1

    def _reader(self):
        for line in self.p.stdout:
            line = line.strip()
            if line:
                self.q.put(line)

    def _errpump(self):
        for line in self.p.stderr:
            self.err.append(line)

    def send(self, obj):
        self.p.stdin.write(json.dumps(obj) + "\n")
        self.p.stdin.flush()

    def req(self, method, params=None, timeout=180):
        i = self.next_id; self.next_id += 1
        msg = {"jsonrpc": "2.0", "id": i, "method": method}
        if params is not None: msg["params"] = params
        self.send(msg)
        while True:
            try:
                line = self.q.get(timeout=timeout)
            except queue.Empty:
                raise TimeoutError(f"{method} timed out; stderr: {''.join(self.err[-5:])}")
            data = json.loads(line)
            if data.get("id") == i:
                return data

    def notify(self, method, params=None):
        msg = {"jsonrpc": "2.0", "method": method}
        if params is not None: msg["params"] = params
        self.send(msg)

    def handshake(self):
        r = self.req("initialize", {"protocolVersion": "2024-11-05", "capabilities": {},
            "clientInfo": {"name": "zcode-team-ctl", "version": "1.0"}}, timeout=60)
        if "error" in r: raise RuntimeError(r["error"])
        self.notify("notifications/initialized")
        return r

    def call(self, name, args, timeout=900):
        return self.req("tools/call", {"name": name, "arguments": args}, timeout=timeout)

    def close(self):
        try: self.p.kill()
        except Exception: pass

def main():
    if len(sys.argv) < 2:
        print(__doc__); return 2
    m = Mcp()
    try:
        m.handshake()
        if sys.argv[1] == "list":
            r = m.req("tools/list", timeout=60)
            for t in r.get("result", {}).get("tools", []):
                print(f"- {t['name']}: {(t.get('description') or '')[:80]}")
        elif sys.argv[1] == "call":
            args = json.loads(sys.argv[3]) if len(sys.argv) > 3 else {}
            out = m.call(sys.argv[2], args)
            print(json.dumps(out, indent=1, ensure_ascii=False)[:24000])
        else:
            print(__doc__); return 2
    finally:
        m.close()
    return 0

if __name__ == "__main__":
    sys.exit(main())
