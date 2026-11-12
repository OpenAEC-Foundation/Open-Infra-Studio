#!/usr/bin/env python3
"""End-to-end test of assistant proposals and the activity log (#92),
through the MCP server, as an AI assistant would work.

Runs its own Open CAD Studio (REST on --port, isolated configuration folder
``XDG_CONFIG_HOME`` under ``target/e2e-assistent``), starts
``mcp/open_infra_mcp.py`` against it (``OI_REST_PORT``) with the client name
``assistent-e2e`` and ``OI_MCP_REVIEW=1`` (this scripted test plays the user
who reviews), in propose mode.

Steps
  tools        every tool has a class and the MCP annotations.
  site         oi_site Dordrecht (logged, nothing to propose).
  propose      civ_tin (AHN) and civ_alignment: two pending proposals with
               their frames on OI-ASSISTANT-PROPOSAL.
  reject       the alignment: its entities are gone, the TIN stays.
  accept       the TIN: its markers go, the entities stay.
  modify       a feature line (accepted), then civ_feature_line_edit raises it:
               a proposal that modifies the line, removes its old labels and
               adds new ones; reject brings the line and the old labels back,
               with their own handles.
  log          activity_log: each step with client, tool and state; the user's
               accepts and rejects.
  reopen       one more proposal, oi_save, oi_open of a copy: the proposal and
               the log are still there, not stale.
  stale        the user moves an entity of that proposal: stale, reject is
               refused, accept works.
  captures     PNGs of the proposals, after review, the raised line, and the
               Assistant page of the workbench window.

Usage
  python3 tests/e2e/assistent_e2e.py [--ocs PATH] [--port 18822] [--no-install]

The report goes to target/e2e-assistent/rapport.md.
"""

import argparse
import base64
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ocs_e2e  # noqa: E402

REPO = ocs_e2e.REPO
OUT = REPO / "target" / "e2e-assistent"
SERVER = REPO / "mcp" / "open_infra_mcp.py"
CLIENT = {"name": "assistent-e2e", "version": "1"}

# Dordrecht, Sterrenburg: open ground, a small area (AHN at 5 m is quick).
SITE = {"x": 106000, "y": 423300, "width": 300, "height": 240, "name": "Dordrecht assistent"}
AXIS = [[105880, 423220], [105980, 423300], [106100, 423330]]
FEATURE = [[105900, 423350], [105960, 423360], [106020, 423350]]
MARKER_LAYER = "OI-ASSISTANT-PROPOSAL"


def install(xdg):
    env = dict(os.environ, XDG_CONFIG_HOME=str(xdg))
    subprocess.run(["bash", str(REPO / "scripts" / "install.sh")], check=True, env=env, cwd=REPO)


class Mcp:
    """The MCP server over stdio, as a client starts it."""

    def __init__(self, env):
        self.p = subprocess.Popen([sys.executable, str(SERVER)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env)
        self.n = 0
        self.info = self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": CLIENT})
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.p.stdin.flush()

    def rpc(self, method, params=None):
        self.n += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params or {}}) + "\n")
        self.p.stdin.flush()
        return json.loads(self.p.stdout.readline())

    def call(self, name, args=None, image=None):
        r = self.rpc("tools/call", {"name": name, "arguments": args or {}})["result"]
        text = next((c["text"] for c in r["content"] if c["type"] == "text"), "")
        for c in r["content"]:
            if c["type"] == "image" and image:
                image.write_bytes(base64.b64decode(c["data"]))
        return not r.get("isError"), r.get("structuredContent") or {}, text

    def close(self):
        self.p.stdin.close()
        try:
            self.p.wait(5)
        except subprocess.TimeoutExpired:
            self.p.kill()


def window_shot(title, path, size=(1280, 900)):
    """A PNG of an X11 window by its title (the workbench window): moved
    and sized to fit the screen, its client area from xwininfo."""
    try:
        from PIL import ImageGrab
    except ImportError:
        return False, "no Pillow"
    out = subprocess.run(["wmctrl", "-l"], capture_output=True, text=True).stdout
    wid = next((line.split()[0] for line in out.splitlines() if title in line), None)
    if not wid:
        return False, f"no window '{title}'"
    subprocess.run(["wmctrl", "-i", "-r", wid, "-e", f"0,60,40,{size[0]},{size[1]}"])
    subprocess.run(["wmctrl", "-i", "-a", wid])
    time.sleep(2.0)
    info = subprocess.run(["xwininfo", "-id", wid], capture_output=True, text=True).stdout
    val = lambda key: int(next(l.split(":")[1] for l in info.splitlines() if l.strip().startswith(key)))
    x, y, w, h = val("Absolute upper-left X"), val("Absolute upper-left Y"), val("Width"), val("Height")
    img = ImageGrab.grab(bbox=(x, y, x + w, y + h), xdisplay=os.environ.get("DISPLAY", ":0"))
    img.save(path)
    return True, f"{path.name} {w}×{h}"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--ocs", default=os.environ.get("OCS_BIN", ocs_e2e.default_ocs()))
    ap.add_argument("--port", type=int, default=18822)
    ap.add_argument("--no-install", action="store_true", help="use the plugins already installed in the isolated folder")
    args = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    xdg = OUT / "xdg"
    if not args.no_install:
        install(xdg)
    os.environ["XDG_CONFIG_HOME"] = str(xdg)
    conf = xdg / "OpenCADStudio" / "open-infra"
    conf.mkdir(parents=True, exist_ok=True)
    (conf / "settings.json").write_text(json.dumps({"language": "en", "assistant": "propose"}))
    for old in OUT.glob("*.png"):
        old.unlink()
    ocs_e2e.OUT = OUT
    ocs = ocs_e2e.Ocs(args.ocs, args.port)
    env = dict(os.environ, OI_REST_PORT=str(args.port), OCS_API_TOKEN=ocs.token, XDG_CONFIG_HOME=str(xdg), OI_MCP_REVIEW="1")
    env.pop("OI_ASSISTANT", None)
    mcp = Mcp(env)
    results = []
    state = {}

    def step(name, fn):
        t = time.time()
        try:
            ok, detail = fn()
        except Exception as e:  # noqa: BLE001 - the report wants every failure
            ok, detail = False, f"{type(e).__name__}: {e}"
        results.append((name, ok, str(detail), time.time() - t))
        print(("✔" if ok else "✘"), name, "—", str(detail)[:300], flush=True)
        return ok

    def exists(h):
        r = ocs.get(f"/entities/{h}")
        e = (r.get("entities") or [r.get("entity")])[0] if (r.get("entities") or r.get("entity")) else None
        return bool(e) and r.get("ok", True) is not False

    def markers():
        return ocs.handles(MARKER_LAYER) if MARKER_LAYER in ocs.layers() else []

    def call(name, a=None, image=None):
        ok, data, text = mcp.call(name, a, image)
        if not ok:
            raise RuntimeError(f"{name}: {text[:300]}")
        return data, text

    def pending():
        data, _ = call("proposals_list")
        return {p["id"]: p for p in data.get("proposals", [])}

    def capture(name, region=None):
        path = OUT / f"{name}.png"
        a = {"path": str(path), "max_dimension": 1600}
        if region:
            a["region"] = region
        call("oi_view", {"preset": "TOP", "style": "WIREFRAME2D"})
        call("oi_capture", a)
        return path.exists() and path.stat().st_size > 0, path.name

    area = [SITE["x"] - SITE["width"] / 2 - 20, SITE["y"] - SITE["height"] / 2 - 20, SITE["x"] + SITE["width"] / 2 + 20, SITE["y"] + SITE["height"] / 2 + 40]

    def tools():
        listed = mcp.rpc("tools/list")["result"]["tools"]
        kinds = {t["name"]: t.get("_meta", {}).get("org.openaec/class") for t in listed}
        missing = [n for n, k in kinds.items() if k not in ("read", "edit", "export", "save")]
        no_hints = [t["name"] for t in listed if "readOnlyHint" not in t.get("annotations", {})]
        count = {k: list(kinds.values()).count(k) for k in ("read", "edit", "export", "save")}
        return not missing and not no_hints and "proposal_accept" in kinds, f"{len(listed)} tools {count}; unclassified {missing}"

    def site():
        ok, data, text = mcp.call("oi_site", SITE)
        if not ok:
            return False, text
        time.sleep(1.5)
        act = data.get("activity", {})
        return act.get("state") == "done" and "proposal" not in data, f"{text.splitlines()[0][:120]}; log state {act.get('state')}"

    def propose():
        tin, text = call("civ_tin", {"source": "AHN", "spacing": 5})
        p1 = tin.get("proposal", {})
        state["tin"] = p1.get("handles", [])
        al, text2 = call("civ_alignment", {"pis": AXIS, "radii": [60], "name": "Assistent As"})
        p2 = al.get("proposal", {})
        state["axis"] = p2.get("handles", [])
        state["axis_handle"] = al.get("as")
        m = markers()
        ok = p1.get("id") == "P1" and p2.get("id") == "P2" and p1.get("state") == "pending" and len(state["tin"]) > 0 and len(state["axis"]) > 0 and len(m) == 4
        ok = ok and "waiting for the user's review" in text and all(exists(h) for h in state["tin"][:5] + state["axis"])
        return ok, f"P1 TIN +{p1.get('added')}, P2 alignment +{p2.get('added')}, {len(m)} markers"

    def listed():
        ps = pending()
        ok = set(ps) == {"P1", "P2"} and not any(p["stale"] for p in ps.values()) and ps["P2"]["client"] == "assistent-e2e 1"
        return ok, ", ".join(f"{i} {p['title']} stale={p['stale']} restorable={p['restorable']}" for i, p in ps.items())

    def reject_alignment():
        _, text = call("proposal_reject", {"id": "P2"})
        gone = [h for h in state["axis"] if exists(h)]
        tin_there = all(exists(h) for h in state["tin"])
        ps = pending()
        return not gone and tin_there and set(ps) == {"P1"} and len(markers()) == 2, f"{text.splitlines()[0][:140]}; alignment left {len(gone)}, TIN there {tin_there}"

    def accept_tin():
        _, text = call("proposal_accept", {"id": "P1"})
        ps = pending()
        return not ps and not markers() and all(exists(h) for h in state["tin"]), f"{text.splitlines()[0][:140]}; markers {len(markers())}"

    def modify_and_reject():
        fl, _ = call("civ_feature_line", {"points": FEATURE, "heights": "MAAIVELD", "name": "Assistent kantlijn"})
        h = fl.get("kantlijn")
        call("proposal_accept", {"id": fl["proposal"]["id"]})
        before = ocs.entity(h)
        before_z = fl.get("hoogtes") or []
        ed, _ = call("civ_feature_line_edit", {"line": h, "action": "raise", "dz": 1.5})
        p = ed.get("proposal", {})
        raised = ed.get("hoogtes") or []
        lifted = len(raised) == len(before_z) and all(abs(b - a - 1.5) < 1e-6 for a, b in zip(before_z, raised))
        ps = pending()
        q = ps.get(p.get("id"), {})
        state["modify"] = (p.get("id"), q.get("modified", []), q.get("removed", []), q.get("added", []))
        capture("kantlijn-verhoogd", area)
        _, text = call("proposal_reject", {"id": p.get("id")})
        after = ocs.entity(h)
        old_labels_back = all(exists(x) for x in q.get("removed", []))
        new_labels_gone = not any(exists(x) for x in q.get("added", []))
        same = after == before
        ok = p.get("modified", 0) >= 1 and lifted and same and old_labels_back and new_labels_gone and h in q.get("modified", [])
        return ok, (f"{p.get('id')}: ~{p.get('modified')} −{p.get('removed')} +{p.get('added')}, raised {lifted}; after reject: line as before {same}, "
                    f"old labels back {old_labels_back} ({len(q.get('removed', []))}), new labels gone {new_labels_gone}")

    def log():
        data, text = call("activity_log", {"max": 100})
        es = data.get("entries", [])
        by = lambda tool: [e for e in es if e["tool"] == tool]
        tin = by("civ_tin")
        axis = by("civ_alignment")
        edit = by("civ_feature_line_edit")
        reviews = [e for e in es if e["class"] == "review"]
        ok = (tin and tin[0]["state"] == "accepted" and tin[0]["who"] == "assistent-e2e 1" and tin[0]["added"] > 0
              and axis and axis[0]["state"] == "rejected" and edit and edit[0]["state"] == "rejected" and edit[0]["modified"] >= 1
              and len(reviews) >= 4 and all("OI_MCP_REVIEW" in e["who"] for e in reviews) and by("oi_site"))
        lines = [f"{e['tool']}:{e['state']}" for e in reversed(es)]
        return bool(ok), f"{len(es)} lines: {', '.join(lines)}"

    def reopen():
        al, _ = call("civ_alignment", {"pis": [[105900, 423180], [106080, 423200]], "name": "Blijft openstaan"})
        p = al.get("proposal", {})
        state["kept"] = (p.get("id"), p.get("handles", []))
        before_log = len(call("activity_log", {"max": 500})[0].get("entries", []))
        saved = OUT / "assistent.dwg"
        copy = OUT / "assistent-heropend.dwg"
        for f in (saved, copy):
            if f.exists():
                f.unlink()
        call("oi_save", {"path": str(saved)})
        time.sleep(2)
        shutil.copy(saved, copy)
        call("oi_open", {"path": str(copy)})
        time.sleep(3)
        ps = pending()
        q = ps.get(p.get("id"), {})
        after = call("activity_log", {"max": 500})[0].get("entries", [])
        noted = any(e["tool"] == "oi_save" for e in after)
        ok = bool(q) and not q["stale"] and q["restorable"] and len(after) >= before_log and noted and len(markers()) == 2
        return ok, f"{p.get('id')} after reopening: stale={q.get('stale')}, restorable={q.get('restorable')}; log {before_log}→{len(after)} lines, save noted {noted}"

    def stale():
        pid, hs = state["kept"]
        h = next(x for x in hs if exists(x))
        r = ocs.call("/entities/transform", {"handles": [h], "action": "move", "vector": [5.0, 0.0, 0.0], "document_id": ocs.doc()})
        q = pending().get(pid, {})
        ok_reject, _, text = mcp.call("proposal_reject", {"id": pid})
        _, acc = call("proposal_accept", {"id": pid})
        ok = q.get("stale") is True and not ok_reject and "edited since" in text and not pending() and not markers()
        return ok, f"moved {h} ({r.get('error') or 'ok'}): stale={q.get('stale')}; reject refused: {text.splitlines()[0][:110]}; then {acc.splitlines()[0][:80]}"

    def page():
        call("oi_command", {"command": "CIV_ASSISTENT"})
        time.sleep(4)
        return window_shot("Open Infra — Civiel", OUT / "assistent-pagina.png")

    try:
        def sessions():
            # oi_sessions answers in its text (JSON), without structured data.
            _, text = call("oi_sessions")
            selected = json.loads(text).get("selected") or ""
            return selected == f"rest:{args.port}", f"selected {selected}"

        step("sessions", sessions)
        step("tools: classes and annotations", tools)
        step("site (logged, no proposal)", site)
        if step("propose a TIN and an alignment", propose):
            step("capture: two proposals", lambda: capture("voorstellen", area))
            step("proposals_list", listed)
            step("reject the alignment, the TIN stays", reject_alignment)
            step("accept the TIN", accept_tin)
            step("capture: after review", lambda: capture("na-beoordeling", area))
        step("modify existing entities, reject: the old ones are back", modify_and_reject)
        step("activity log", log)
        step("save, reopen: proposals and log survive", reopen)
        step("stale: edited since, reject refused, accept", stale)
        step("capture: the Assistant page", page)
    finally:
        mcp.close()
        ocs.stop()
    md = ["# Assistant proposals e2e (#92)", "", "| step | result | s | detail |", "|---|---|---|---|"]
    for name, ok, detail, sec in results:
        md.append(f"| {name} | {'ok' if ok else 'FAIL'} | {sec:.1f} | {detail.replace('|', '/')} |")
    failed = [r for r in results if not r[1]]
    md += ["", f"{len(results) - len(failed)}/{len(results)} steps ok."]
    (OUT / "rapport.md").write_text("\n".join(md) + "\n")
    print(f"\n{len(results) - len(failed)}/{len(results)} steps ok; report {OUT / 'rapport.md'}")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
