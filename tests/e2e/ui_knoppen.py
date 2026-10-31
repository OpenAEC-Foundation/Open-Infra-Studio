#!/usr/bin/env python3
"""Click every button in the Open Infra windows (GIS and Civiel), in a real
Open CAD Studio, and report what each one did.

The plugins are built with the `ui-test` feature, which puts their egui windows
on the accessibility bus (AT-SPI). The script runs Open CAD Studio in its own
D-Bus session, with its own accessibility bus and its own config folder, so
your desktop settings and your installed plugins stay as they are:

    CARGO_TARGET_DIR=target/ui-test cargo build --release -p open-infra --features ui-test
    CARGO_TARGET_DIR=target/ui-test cargo build --release -p open-infra-civiel --features ui-test
    python3 tests/e2e/ui_knoppen.py [--ocs <AppImage>] [--venster gis|civiel] [--pagina <naam>]

Per page it lists the buttons, clicks each enabled one and reports what
happened: a new status text, another page, a closed window, or nothing.
Checkboxes are switched on and back off. The report is in
target/e2e/ui/knoppen.md (and .json). Needs wmctrl and the AT-SPI Python
bindings (gir1.2-atspi-2.0).
"""

import argparse
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from ocs_e2e import DATA, default_ocs

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "target" / "e2e" / "ui"
LIB = ROOT / "target" / "ui-test" / "release"

# Small import area in Dordrecht, so import buttons finish quickly.
SITE = "GIS_SITE RD 104900 425350 250 200 UI-test"
# Window title, the heading that identifies it on the accessibility bus
# (egui does not pass the title on), and the command that opens it.
WINDOWS = {
    "gis": ("Open Infra — GIS-workbench", "Open Infra · GIS", "GIS_LOCATIE"),
    "civiel": ("Open Infra — Civiel", "Open Infra · Civiel", "CIV_INSTELLINGEN"),
}
# A running job: a verb with an ellipsis ("Importeren…", "OpenStreetMap ophalen via …").
BUSY = re.compile(r"(bezig|laden|ophalen|importeren|berekenen|controleren|zoeken|maken|herstellen)\b.{0,80}…", re.I)
NAV_X = 200  # the navigation column is 196 px wide


# ---------------------------------------------------------------- outer run

def prepare(a):
    """Config folder with the ui-test plugins, and an empty drawing."""
    cfg = OUT / "cfg"
    shutil.rmtree(cfg, ignore_errors=True)
    rustc = subprocess.run(["rustc", "--version"], capture_output=True, text=True).stdout.strip()
    lock = (ROOT / "Cargo.lock").read_text()
    i = lock.index('name = "acadrust"')
    acad = lock[i:].split('source = "', 1)[1].split('"', 1)[0]
    for pid, lib, toml in (("openaec.open-infra", "libopen_infra.so", ROOT / "plugin.toml"),
                           ("openaec.open-infra-civiel", "libopen_infra_civiel.so", ROOT / "civiel" / "plugin.toml")):
        src = LIB / lib
        if not src.exists():
            sys.exit(f"{src} ontbreekt: bouw eerst met --features ui-test (zie de uitleg bovenin)")
        dest = cfg / "OpenCADStudio" / "plugins" / pid
        dest.mkdir(parents=True)
        shutil.copy2(src, dest / lib)
        (dest / "plugin.toml").write_text(toml.read_text().replace("__RUSTC_VERSION__", rustc).replace("__ACADRUST_SOURCE__", acad))
    drawing = OUT / "ui-test.dwg"
    shutil.copy2(a.tekening or DATA / "leeg-m.dwg", drawing)
    return cfg, drawing


def stop_inner():
    """dbus-run-session does not pass signals on: stop the inner run itself."""
    try:
        os.kill(int((OUT / "inner.pid").read_text()), signal.SIGTERM)
    except (OSError, ValueError):
        pass


def outer(a):
    OUT.mkdir(parents=True, exist_ok=True)
    cfg, drawing = prepare(a)
    env = dict(os.environ, OI_UI_INNER="1", XDG_CONFIG_HOME=str(cfg), OCS_V4_SNAPSHOT_SEGMENT_SIZE="268435456")
    cmd = ["dbus-run-session", "--", sys.executable, __file__] + sys.argv[1:] + ["--drawing", str(drawing)]
    # The private D-Bus session is chatty (portals, keyring): its log goes to a file.
    with open(OUT / "dbus.log", "w") as log:
        child = subprocess.Popen(cmd, env=env, stderr=log)
        # Stopping this script stops the test run (and its Open CAD Studio).
        signal.signal(signal.SIGTERM, lambda *_: stop_inner())
        signal.signal(signal.SIGINT, lambda *_: stop_inner())
        code = child.wait()
    if code not in (0, 1):
        text = (OUT / "dbus.log").read_text(errors="replace")
        i = text.rfind("Traceback")
        print(text[i:] if i >= 0 else text[-2000:], file=sys.stderr)
    sys.exit(code)


# ---------------------------------------------------------------- inner run

def a11y_bus():
    """Our own accessibility bus in this D-Bus session, switched on."""
    launcher = subprocess.Popen(["/usr/libexec/at-spi-bus-launcher", "--launch-immediately"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(50):
        r = subprocess.run(["busctl", "--user", "set-property", "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Status", "IsEnabled", "b", "true"], capture_output=True)
        if r.returncode == 0:
            return launcher
        time.sleep(0.2)
    sys.exit("de toegankelijkheidsbus start niet (at-spi-bus-launcher)")


class Ocs:
    """Our Open CAD Studio: the process group and the automation session."""

    def __init__(self, a):
        self.log = OUT / "ocs.log"
        self.p = subprocess.Popen([str(a.ocs or default_ocs()), a.drawing], stdout=open(self.log, "w"), stderr=subprocess.STDOUT, start_new_session=True)
        import open_infra_mcp as mcp  # after XDG_CONFIG_HOME is set
        self.mcp = mcp.OCS
        t0 = time.time()
        while time.time() - t0 < 90:
            try:
                self.mcp.pick()
                self.mcp.document_id()
                return
            except Exception:
                time.sleep(1)
        sys.exit("Open CAD Studio start niet (zie target/e2e/ui/ocs.log)")

    def pids(self):
        out = subprocess.run(["ps", "-eo", "pid=,pgid="], capture_output=True, text=True).stdout.split()
        return {out[i] for i in range(0, len(out), 2) if out[i + 1] == str(self.p.pid)}

    def visible(self, title):
        """Is our window with this title on screen (mapped)?"""
        pids = self.pids()
        out = subprocess.run(["wmctrl", "-lp"], capture_output=True, text=True).stdout
        return any(len(f) >= 5 and f[2] in pids and title in f[4] for f in (line.split(None, 4) for line in out.splitlines()))

    def picture(self, title):
        """The window's pixels (to see changes on the map canvas)."""
        pids = self.pids()
        out = subprocess.run(["wmctrl", "-lp"], capture_output=True, text=True).stdout
        ids = [int(f[0], 16) for f in (line.split(None, 4) for line in out.splitlines()) if len(f) >= 5 and f[2] in pids and title in f[4]]
        if not ids:
            return None
        try:
            from Xlib import X, display
            d = display.Display()
            w = d.create_resource_object("window", ids[0])
            g = w.get_geometry()
            return w.get_image(0, 0, g.width, g.height, X.ZPixmap, 0xFFFFFFFF).data
        except Exception:
            return None

    def cancel(self):
        try:
            self.mcp.request("cancel", {"document_id": self.mcp.document_id()}, timeout=10)
        except Exception:
            pass

    def crashed(self):
        log = self.log.read_text(errors="replace")
        for word in ("panicked", "plugin crashed", "exited unexpectedly", "SIGSEGV", "disconnected"):
            if word in log:
                return word
        return None

    def stop(self):
        try:
            os.killpg(self.p.pid, 15)
        except ProcessLookupError:
            pass


class Ui:
    """The Open Infra windows as seen over AT-SPI."""

    def __init__(self):
        import gi
        gi.require_version("Atspi", "2.0")
        from gi.repository import Atspi
        self.A = Atspi

    def frame(self, marker, timeout=20):
        """The window whose tree holds the heading `marker`."""
        t0 = time.time()
        while time.time() - t0 < timeout:
            desk = self.A.get_desktop(0)
            for i in range(desk.get_child_count()):
                app = desk.get_child_at_index(i)
                for j in range(app.get_child_count() if app else 0):
                    w = app.get_child_at_index(j)
                    if w is not None and any(n["text"] == marker for n in self.nodes(w)):
                        return w
            time.sleep(0.3)
        return None

    def nodes(self, root):
        """All nodes below `root` as dicts: role, name, text, enabled, toggled, x, y, node."""
        A = self.A
        out = []
        stack = [root]
        while stack:
            n = stack.pop()
            try:
                role = n.get_role_name()
                name = n.get_name() or ""
                states = n.get_state_set()
                try:
                    e = n.get_extents(A.CoordType.WINDOW)
                    x, y = e.x, e.y
                except Exception:
                    x = y = -1
                text = name
                if not text and role == "label":
                    try:
                        text = n.get_text(0, -1) or ""
                    except Exception:
                        pass
                value = ""
                if role in ("spin button", "text", "entry", "slider", "combo box", "password text"):
                    try:
                        value = n.get_text(0, -1) or ""
                    except Exception:
                        pass
                    if not value and role in ("spin button", "slider"):
                        try:
                            value = f"{n.get_current_value():g}"
                        except Exception:
                            pass
                out.append({
                    "role": role, "name": name, "text": text, "value": value, "x": x, "y": y, "node": n,
                    "enabled": states.contains(A.StateType.ENABLED) or states.contains(A.StateType.SENSITIVE),
                    "toggled": states.contains(A.StateType.CHECKED) or states.contains(A.StateType.PRESSED),
                })
                kids = [n.get_child_at_index(i) for i in range(n.get_child_count())]
                stack.extend(k for k in reversed(kids) if k is not None)
            except Exception:
                continue
        return out

    def click(self, node):
        n = node.get_n_actions()
        for i in range(n):
            if node.get_action_name(i) in ("click", "press", "activate"):
                return node.do_action(i)
        return node.do_action(0) if n else False


def is_button(n):
    return n["role"] in ("push button", "toggle button", "button")


def label_of(n):
    return n["name"] or f"(zonder naam: {n['role']} op {n['x']},{n['y']})"


def texts(nodes):
    return [n["text"] for n in nodes if n["role"] == "label" and n["text"]]


def status_of(nodes):
    """The status line: the lowest label in the window."""
    labels = [n for n in nodes if n["role"] == "label" and n["text"]]
    return max(labels, key=lambda n: n["y"])["text"] if labels else ""


def signature(nodes):
    """Everything visible in the page area, to see whether a click changed it."""
    return [(n["role"], n["name"], n["text"], n["value"], n["toggled"], n["enabled"]) for n in nodes if n["x"] >= NAV_X]


def current_page(nodes, pages):
    sel = [n["name"] for n in nodes if is_button(n) and n["x"] < NAV_X and n["toggled"] and n["name"] in pages]
    return sel[0] if sel else None


class Tester:
    def __init__(self, a, ocs, ui):
        self.a, self.ocs, self.ui = a, ocs, ui
        self.results = []

    def window(self, key, timeout=3):
        title, marker, cmd = WINDOWS[key]
        if self.ocs.visible(title):
            w = self.ui.frame(marker, timeout=timeout)
            if w is not None:
                return w
        self.ocs.mcp.host(cmd)
        w = self.ui.frame(marker)
        if w is None:
            raise RuntimeError(f"venster '{title}' staat niet op de toegankelijkheidsbus (gebouwd met --features ui-test?)")
        return w

    def settle(self, w, timeout):
        """Wait until no job runs; return the nodes and whether one still runs."""
        t0 = time.time()
        time.sleep(0.7)
        while True:
            nodes = self.ui.nodes(w)
            busy = any(BUSY.search(t) for t in texts(nodes))
            if not busy or time.time() - t0 > timeout:
                return nodes, busy
            time.sleep(0.5)

    def goto(self, key, page):
        """Open `page` and wait until the window shows it."""
        w = self.window(key)
        for n in self.ui.nodes(w):
            if is_button(n) and n["x"] < NAV_X and n["name"] == page:
                for _ in range(2):
                    if current_page(self.ui.nodes(w), [page]) == page:
                        break
                    # A second click when the first only closed a pop-up.
                    self.ui.click(n["node"])
                    for _ in range(15):
                        time.sleep(0.4)
                        if current_page(self.ui.nodes(w), [page]) == page:
                            break
                time.sleep(0.5)
                return w
        raise RuntimeError(f"pagina {page} niet gevonden")

    def run_window(self, key):
        title = WINDOWS[key][0]
        w = self.window(key)
        nodes = self.ui.nodes(w)
        # The language and Dutch-modules switches sit in the navigation column too.
        switches = {"NL", "EN", "aan", "uit", "on", "off"}
        pages = [n["name"] for n in nodes if is_button(n) and 0 <= n["x"] < NAV_X and n["name"] and n["name"] not in switches]
        print(f"\n=== {title}: {len(pages)} pagina's: {', '.join(pages)}", flush=True)
        for page in pages:
            if self.a.pagina and self.a.pagina.lower() not in page.lower():
                continue
            self.run_page(key, page, pages)

    def run_page(self, key, page, pages):
        title = WINDOWS[key][0]
        w = self.goto(key, page)
        nodes, _ = self.settle(w, 30)
        if current_page(nodes, pages) != page:
            self.add(key, page, page, "navigatie", False, "GEEN EFFECT (pagina opent niet)")
            return
        print(f"\n--- {title} · {page}", flush=True)
        # Drop-down lists first: a choice (a server, a layer) can switch on
        # the buttons below it.
        i = 0
        while not self.a.alleen_lijst:
            combos = [n for n in self.ui.nodes(w) if n["x"] >= NAV_X and n["role"] == "combo box"]
            if i >= len(combos):
                break
            name = f"keuzelijst {i + 1}" + (f" ({combos[i]['value'][:30]})" if combos[i]["value"] else "")
            self.add(key, page, name, "combo box", combos[i]["enabled"], self.choose(key, page, i) if combos[i]["enabled"] else "uitgeschakeld")
            i += 1
        nodes, _ = self.settle(w, 30)
        controls = [n for n in nodes if n["x"] >= NAV_X and (is_button(n) or n["role"] in ("check box", "radio button"))]
        seen = {}
        for c in controls:
            label = label_of(c)
            nth = seen.get(label, 0)
            seen[label] = nth + 1
            if not c["enabled"]:
                outcome = "uitgeschakeld"
            elif self.a.alleen_lijst or any(s.lower() in label.lower() for s in self.a.overslaan):
                outcome = "niet geklikt"
            else:
                outcome = self.click_and_watch(key, page, pages, label, nth, c["role"])
                if outcome.startswith(("opent pagina", "niet meer te vinden", "GEEN EFFECT")):
                    # Once more, to tell a real result from a tree that was still changing.
                    again = self.click_and_watch(key, page, pages, label, nth, c["role"])
                    if again != outcome:
                        outcome = f"wisselend: {outcome} / {again}"
            self.add(key, page, label, c["role"], c["enabled"], outcome)

    def choose(self, key, page, nth):
        """Open drop-down list `nth`, pick another option; keep it when it
        switches buttons on, else put the old one back."""
        w = self.goto(key, page)
        combos = [n for n in self.ui.nodes(w) if n["x"] >= NAV_X and n["role"] == "combo box"]
        if nth >= len(combos):
            return "niet meer te vinden"
        before = self.ui.nodes(w)
        seen = {(n["role"], n["name"], n["x"], n["y"]) for n in before}
        disabled = {label_of(n) for n in before if is_button(n) and not n["enabled"]}
        self.ui.click(combos[nth]["node"])
        time.sleep(0.8)
        options = [n for n in self.ui.nodes(w) if (n["role"], n["name"], n["x"], n["y"]) not in seen and n["name"] and (is_button(n) or n["role"] in ("list item", "menu item", "radio button"))]
        if not options:
            return "GEEN EFFECT (keuzelijst opent niet)"
        old = next((o for o in options if o["toggled"]), None)
        pick = next((o for o in options if not o["toggled"]), options[0])
        self.ui.click(pick["node"])
        after, busy = self.settle(w, 90)
        enabled = {label_of(n) for n in after if is_button(n) and n["enabled"]} & disabled
        result = f"{len(options)} opties, gekozen: {pick['name'][:40]}"
        if enabled:
            return result + f" → zet aan: {', '.join(sorted(enabled))[:60]}"
        if old is not None:
            combos = [n for n in self.ui.nodes(w) if n["x"] >= NAV_X and n["role"] == "combo box"]
            if nth < len(combos):
                self.ui.click(combos[nth]["node"])
                time.sleep(0.8)
                back = [n for n in self.ui.nodes(w) if n["name"] == old["name"] and (is_button(n) or n["role"] in ("list item", "menu item", "radio button"))]
                if back:
                    self.ui.click(back[-1]["node"])
                    self.settle(w, 60)
                    result += f" (terug naar {old['name'][:30]})"
        self.close_popup(w, seen)
        if signature(after) == signature(before) and self.ocs.picture(WINDOWS[key][0]) is None:
            return "GEEN EFFECT (" + result + ")"
        return result

    def close_popup(self, w, seen):
        """A drop-down list still open (nodes that were not there before it
        opened) blocks the rest of the window: click its box to close it."""
        for _ in range(2):
            extra = [n for n in self.ui.nodes(w) if (n["role"], n["name"], n["x"], n["y"]) not in seen and n["name"] and is_button(n)]
            if len(extra) < 2:
                return
            boxes = [n for n in self.ui.nodes(w) if n["role"] == "combo box"]
            for b in boxes:
                self.ui.click(b["node"])
                time.sleep(0.5)
                if len([n for n in self.ui.nodes(w) if (n["role"], n["name"], n["x"], n["y"]) not in seen and n["name"] and is_button(n)]) < 2:
                    return

    def add(self, key, page, label, role, enabled, outcome):
        self.results.append({"venster": key, "pagina": page, "knop": label, "soort": role, "aan": enabled, "uitkomst": outcome})
        print(f"  {label[:58]:58s} {outcome}", flush=True)

    def find(self, w, label, nth):
        hits = [n for n in self.ui.nodes(w) if n["x"] >= NAV_X and label_of(n) == label]
        return hits[nth] if nth < len(hits) else None

    def click_and_watch(self, key, page, pages, label, nth, role):
        title = WINDOWS[key][0]
        w = self.goto(key, page)
        target = self.find(w, label, nth)
        if target is None:
            return "niet meer te vinden"
        before = self.ui.nodes(w)
        before_status, before_texts = status_of(before), set(texts(before))
        before_pic = self.ocs.picture(title)
        if role == "radio button" and target["toggled"]:
            return "al gekozen"
        try:
            self.ui.click(target["node"])
        except Exception as e:
            return f"FOUT bij klikken: {e}"
        time.sleep(0.8)
        # A command waiting for a pick in the drawing.
        state = self.ocs.mcp.request("state")
        picking = state.get("command") or state.get("operation")
        if picking:
            self.ocs.cancel()
        crash = self.ocs.crashed()
        if crash:
            return f"CRASH ({crash})"
        if not self.ocs.visible(title):
            return "sluit het venster" + (f" (opdracht {picking})" if picking else "")
        after, busy = self.settle(w, 120)
        crash = self.ocs.crashed()
        if crash:
            return f"CRASH ({crash})"
        if role in ("check box", "radio button"):
            again = self.find(w, label, nth)
            flipped = again is not None and again["toggled"] != target["toggled"]
            if role == "check box" and flipped:
                self.ui.click(again["node"])
                time.sleep(0.5)
            return "schakelt" if flipped else "GEEN EFFECT (schakelt niet)"
        again = self.find(w, label, nth)
        if again is not None and again["toggled"] != target["toggled"]:
            self.ui.click(again["node"])
            time.sleep(0.5)
            return "schakelt (aan/uit-knop)"
        now = current_page(after, pages)
        status = status_of(after)
        new = [t for t in texts(after) if t not in before_texts]
        if picking:
            return f"start opdracht in de tekening ({picking}), afgebroken"
        if busy:
            return f"loopt na 120 s nog: {status[:80]}"
        if now and now != page:
            return f"opent pagina {now}"
        if status != before_status:
            bad = re.search(r"mislukt|\berror\b|kan niet|kon niet|geen tekening actief|panicked", status.lower()) is not None
            return ("FOUT: " if bad else "melding: ") + status[:110]
        if new:
            return "toont: " + " | ".join(new)[:110]
        if signature(after) != signature(before):
            changed = [f"{a[1] or a[0]}: {a[3]}" for a, b in zip(signature(after), signature(before)) if a != b and a[3]]
            return "scherm verandert" + (f" ({'; '.join(changed)[:90]})" if changed else "")
        pic = self.ocs.picture(title)
        if before_pic is not None and pic is not None and pic != before_pic:
            return "beeld verandert (kaart of tekening)"
        if label == "Zoom naar gebied":
            return "geen verandering (de kaart stond al op het gebied)"
        if status and status != "Klaar.":
            return f"GEEN EFFECT, of dezelfde melding als hiervoor: {status[:90]}"
        return "GEEN EFFECT"


def report(results):
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "knoppen.json").write_text(json.dumps(results, ensure_ascii=False, indent=1))
    lines = ["# Knoppentest Open Infra", "", "| Venster | Pagina | Knop | Uitkomst |", "|---|---|---|---|"]
    for r in results:
        lines.append(f"| {r['venster']} | {r['pagina']} | {r['knop']} | {r['uitkomst']} |")
    bad = problems(results)
    lines += ["", f"{len(results)} elementen, {len(bad)} met een probleem."]
    (OUT / "knoppen.md").write_text("\n".join(lines) + "\n")
    print(f"\n{len(results)} elementen getest, {len(bad)} met een probleem; rapport: {OUT / 'knoppen.md'}")


def problems(results):
    return [r for r in results if r["uitkomst"].startswith(("FOUT", "CRASH", "GEEN EFFECT"))]


def inner(a):
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    (OUT / "inner.pid").write_text(str(os.getpid()))
    launcher = a11y_bus()
    ocs = Ocs(a)
    t = Tester(a, ocs, Ui())
    try:
        time.sleep(3)
        ocs.mcp.run(SITE, timeout=60)
        for key in ([a.venster] if a.venster else list(WINDOWS)):
            t.run_window(key)
    finally:
        report(t.results)
        ocs.stop()
        launcher.terminate()
    sys.exit(1 if problems(t.results) else 0)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--ocs", help="Open CAD Studio (AppImage of binary)")
    ap.add_argument("--venster", choices=list(WINDOWS), help="alleen dit venster")
    ap.add_argument("--pagina", help="alleen pagina's met deze tekst in de naam")
    ap.add_argument("--tekening", help="begin met een kopie van deze tekening")
    ap.add_argument("--overslaan", action="append", default=[], help="knoppen met deze tekst niet klikken (mag vaker)")
    ap.add_argument("--alleen-lijst", action="store_true", help="knoppen alleen opsommen, niet klikken")
    ap.add_argument("--drawing", help=argparse.SUPPRESS)
    a = ap.parse_args()
    if os.environ.get("OI_UI_INNER"):
        sys.path.insert(0, str(ROOT / "mcp"))
        inner(a)
    else:
        outer(a)


if __name__ == "__main__":
    main()
