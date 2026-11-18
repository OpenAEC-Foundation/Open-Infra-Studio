"""Catch literal labels and tooltips passed directly to common workbench widgets."""

import re
import unittest
from pathlib import Path

UI = Path(__file__).resolve().parents[1] / "src" / "ui"
LITERAL = r'"((?:\\.|[^"\\])*)"'
TEXT_CALL = re.compile(
    r'\b(?:label|heading|button|small_button|hint_text|on_hover_text|'
    r'on_disabled_hover_text|selected_text|primary_button|'
    r'Button::new|RichText::new|CollapsingHeader::new)\s*\(\s*' + LITERAL
)
# The workbench's own page and group helpers take their labels after the UI
# handle (and the icon identifier for a page).
GROUP_TITLE = re.compile(r'\bgroup\(\s*(?:ui|col|&mut cols\[\d+\]),\s*' + LITERAL)
PAGE_TITLE = re.compile(r'\bpage_header\(\s*ui,\s*"[^"]+",\s*' + LITERAL)
PAGE_SUBTITLE = re.compile(r'\bpage_header\(\s*ui,\s*"[^"]+",\s*(?:tr\([^)]*\)|"[^"]*"),\s*' + LITERAL)
# Technical identifiers, literal symbols and brand names are language neutral.
UNCHANGED = {
    "CRS", "G0505", "Interval", "Model", "OK", "PDOK", "Pixels", "Project", "URL",
    "WFS", "WMS", "XYZ", "in / out / branch / left / right", "x,y,z",
    "https://…/wms  ·  https://…/wfs  ·  https://…/{z}/{x}/{y}.png",
}


class WorkbenchTranslations(unittest.TestCase):
    def test_widget_text_uses_the_translation_route(self):
        literals = []
        for path in sorted(UI.glob("*.rs")):
            source = path.read_text(encoding="utf-8")
            for pattern in (TEXT_CALL, GROUP_TITLE, PAGE_TITLE, PAGE_SUBTITLE):
                for match in pattern.finditer(source):
                    label = match[1]
                    if label in UNCHANGED or not any(char.isalpha() for char in label):
                        continue
                    line = source.count("\n", 0, match.start()) + 1
                    literals.append(f"{path.name}:{line}: {label}")
        self.assertFalse(literals, "Pass workbench text through tr():\n" + "\n".join(literals))


if __name__ == "__main__":
    unittest.main()
