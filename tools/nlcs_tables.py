#!/usr/bin/env python3
"""Regenerate assets/nlcs/*.tsv from the official NLCS publication.

Source: https://github.com/nl-digigo/NLCS (tabellen/publicatie), CC-BY-4.0,
DigiGO / Stichting NLCS. Run with the downloaded tables in a directory:

    tools/nlcs_tables.py <dir-with-csv-and-xls> assets/nlcs
"""
import csv, glob, os, sys

import xlrd  # NLCS-BGT mapping is published as .xls


def rows(path):
    """The publication CSVs quote every row as one field with doubled quotes."""
    out = []
    for line in open(path, encoding="utf-8", errors="replace"):
        line = line.rstrip("\n")
        if not line.strip():
            continue
        if line.startswith('"') and line.endswith('"'):
            line = line[1:-1]
        out.append(next(csv.reader([line.replace('""', '"')])))
    return out


def write(path, header, data):
    with open(path, "w", encoding="utf-8") as f:
        f.write("\t".join(header) + "\n")
        for r in data:
            f.write("\t".join(str(x).replace("\t", " ").strip() for x in r) + "\n")
    print(f"{path}: {len(data)} rows")


def main(src, dst):
    os.makedirs(dst, exist_ok=True)
    # Objects of all main groups: id, group, name, parent, element types, per-status style.
    objs = []
    for p in sorted(glob.glob(os.path.join(src, "obj", "*.csv"))):
        r = rows(p)
        if not r or r[0][0] != "hoofdgroep":
            continue
        h = {k: i for i, k in enumerate(r[0])}
        for x in r[1:]:
            if len(x) < len(h):
                continue
            g = lambda k: x[h[k]]
            objs.append([g("id_nummer"), g("hoofdgroep"), g("omschrijving"), g("kind_van"), g("element"), g("discipline"),
                         g("lw_b"), g("kl_b"), g("lt_b"), g("lw_n"), g("kl_n"), g("lt_n"),
                         g("lw_t"), g("kl_t"), g("lt_t"), g("lw_v"), g("kl_v"), g("lt_v")])
    write(os.path.join(dst, "objects.tsv"),
          ["id", "group", "name", "parent", "element", "discipline",
           "lw_b", "kl_b", "lt_b", "lw_n", "kl_n", "lt_n", "lw_t", "kl_t", "lt_t", "lw_v", "kl_v", "lt_v"], objs)
    # Colours: NLCS colour number (0-255) -> RGB.
    cols = [[x[3], x[4]] for x in rows(os.path.join(src, "lijnkleuren.csv"))[1:]]
    write(os.path.join(dst, "colors.tsv"), ["number", "hex"], cols)
    write(os.path.join(dst, "linetypes.tsv"), ["name", "group", "lin"],
          [[x[3], x[2], x[6]] for x in rows(os.path.join(src, "lijntypes.csv"))[1:]])
    write(os.path.join(dst, "statuses.tsv"), ["code", "description"],
          [[x[2], x[3]] for x in rows(os.path.join(src, "statussen.csv"))[1:]])
    write(os.path.join(dst, "disciplines.tsv"), ["code", "description"],
          [[x[2], x[4] or x[3]] for x in rows(os.path.join(src, "disciplines.csv"))[1:]])
    write(os.path.join(dst, "groups.tsv"), ["code", "description"],
          [[x[3], x[2]] for x in rows(os.path.join(src, "hoofdgroepen.csv"))[1:]])
    # BGT -> NLCS mapping (IMGeo attribute values -> NLCS object/symbol).
    sh = xlrd.open_workbook(os.path.join(src, "NLCS-BGT-5.0.xls")).sheet_by_index(0)
    hdr = [str(sh.cell_value(0, c)).strip() for c in range(sh.ncols)]
    keep = hdr[:13] + ["NLCS ObjectID", "NLCS SymbolID", "layer(info)", "symbol(info)"]
    data = []
    for r in range(1, sh.nrows):
        row = {hdr[c]: sh.cell_value(r, c) for c in range(sh.ncols)}
        vals = []
        for k in keep:
            v = row.get(k, "")
            if isinstance(v, float):
                v = str(int(v)) if v == int(v) else str(v)
            vals.append(str(v).strip())
        data.append(vals)
    write(os.path.join(dst, "bgt.tsv"), keep, data)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
