#!/usr/bin/env python3
"""Build THIRD-PARTY-NOTICES.txt: the license notice of every package vemry ships.

Run: python3 scripts/gen-notices.py   (needs cargo, and node_modules from `pnpm install`)

The MIT, BSD and Apache licenses of vemry's dependencies ask for their notice to go
with every copy of the software, and MPL-2.0 asks that the recipient be told where
the source is. So the file is bundled into the app (`bundle.resources` in
src-tauri/tauri.conf.json) and opened by the open-licenses command.

What is listed:
- Rust: every crate the app links on the four release targets, that is the normal
  dependency graph of src-tauri. Proc-macros run inside the compiler and leave no
  code in the binary, and build and dev dependencies never reach it, so none of
  those are listed.
- Frontend: the packages reachable from package.json's `dependencies` through their
  own `dependencies`, plus Svelte, whose runtime is compiled into the bundle although
  it is a devDependency. Svelte's dependencies include its compiler's: a package
  listed and then tree-shaken away costs a few lines, one left out is a notice
  missing.

Which text: a package offering a choice of licenses (`MIT OR Apache-2.0`) is listed
under the first one in PREFER that it ships a file for; a package under several at
once (`AND`) with every license file it ships. A NOTICE file always goes in, and so
do the license files of C sources a crate compiles in (VENDORED). A package whose
published files hold no license text gets the canonical text from scripts/licenses/
with its authors as the copyright holders. Identical texts are printed once, under
the list of packages that carry them.

The output is committed. CI regenerates it and diffs, so a dependency change that
forgets to rerun this fails there.
"""
import json
import os
import re
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "THIRD-PARTY-NOTICES.txt")
CANONICAL = os.path.join(ROOT, "scripts", "licenses")
APP_MANIFEST = os.path.join(ROOT, "src-tauri", "Cargo.toml")

# The targets .github/workflows/release.yml builds
TARGETS = ["aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"]

# Under a choice of licenses, the first of these a package offers is the one listed
PREFER = ["MIT", "Apache-2.0", "BSD-3-Clause", "BSD-2-Clause", "ISC", "Zlib", "0BSD", "MIT-0",
          "Unlicense", "CC0-1.0", "BSL-1.0", "Unicode-3.0", "MPL-2.0"]

# Code a crate compiles in from another project, under that project's license
VENDORED = {
    "ring": ["third_party/fiat/LICENSE", "src/polyfill/once_cell/LICENSE-MIT"],
    "rquickjs-sys": ["quickjs/LICENSE"],
}

LICENSE_FILE = re.compile(r"^(licen[cs]e|copying|copyright|notice|unlicense)", re.I)
# A file name that says which license it holds (LICENSE-MIT, LICENSE_APACHE-2.0,
# UNLICENSE…), checked in this order; LICENSE or COPYING alone says nothing.
NAMED = [("NOTICE", "NOTICE"), ("UNLICENSE", "Unlicense"), ("APACHE", "Apache-2.0"), ("MIT", "MIT"),
         ("BSD", "BSD"), ("ZLIB", "Zlib"), ("ISC", "ISC"), ("MPL", "MPL-2.0"), ("BOOST", "BSL-1.0"),
         ("BSL", "BSL-1.0"), ("UNICODE", "Unicode-3.0"), ("CC0", "CC0-1.0")]

# A sentence only the full text of each license has, to tell it from a file that
# merely names the license (compared with the whitespace collapsed)
FINGERPRINT = {
    "MIT": "Permission is hereby granted, free of charge, to any person obtaining a copy",
    "Apache-2.0": "TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION",
    "BSD-3-Clause": "Redistribution and use in source and binary forms",
    "BSD-2-Clause": "Redistribution and use in source and binary forms",
    "ISC": "Permission to use, copy, modify, and/or distribute this software for any",
    "Zlib": "This notice may not be removed or altered from any source distribution",
    "MPL-2.0": "Mozilla Public License Version 2.0",
    "CC0-1.0": "CC0 1.0 Universal",
}

errors = []


def named(filename):
    upper = filename.upper()
    for needle, lid in NAMED:
        if needle in upper:
            return lid
    return None


def family(lid):
    """The name a license file would carry: BSD-3-Clause is filed as LICENSE-BSD."""
    return "BSD" if lid.startswith("BSD") else lid


def options(expr):
    """The license ids of an SPDX expression, and whether all of them apply at once."""
    expr = expr.replace("/", " OR ")
    ids = [re.sub(r"\s+WITH\s+.*", "", t.strip()) for t in re.split(r"\s+(?:OR|AND)\s+|[()]", expr)]
    return [t for t in ids if t], re.search(r"\sAND\s", expr) is not None


def rank(lid):
    return PREFER.index(lid) if lid in PREFER else len(PREFER)


def read(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        lines = [line.rstrip() for line in f.read().replace("\r\n", "\n").replace("\r", "\n").split("\n")]
    return "\n".join(lines).strip("\n")


def canonical(lid, holders):
    path = os.path.join(CANONICAL, f"{lid}.txt")
    if not os.path.exists(path):
        return None
    text = read(path)
    # MIT, BSD and ISC open with a copyright line to fill in; Apache's placeholder
    # sits in its how-to-apply appendix and stays as it is
    return re.sub(r"Copyright (\(c\) )?<year>,? <[^>]+>\.?", f"Copyright (c) {holders}", text, count=1)


def notices(pkg, pkg_dir, expr, license_file, holders, vendored=()):
    """The (label, text) pairs one package contributes."""
    files = sorted(f for f in os.listdir(pkg_dir) if LICENSE_FILE.match(f) and os.path.isfile(os.path.join(pkg_dir, f)))
    notice = [f for f in files if named(f) == "NOTICE"]
    files = [f for f in files if f not in notice]
    out = []
    if not expr and license_file:
        out.append(("LicenseRef", read(os.path.join(pkg_dir, license_file))))
    elif not expr:
        errors.append(f"{pkg}: declares no license")
    else:
        ids, all_of = options(expr)
        spelled = re.sub(r"\s+", " ", re.sub(r"\s*/\s*", " OR ", expr)).strip()

        def label(f):
            match = [lid for lid in ids if named(f) and family(lid) == named(f)]
            return match[0] if match else ids[0] if len(ids) == 1 else spelled

        chosen, picked = [], None
        if all_of:
            chosen = files
        else:
            for lid in sorted(ids, key=rank):
                chosen = [f for f in files if named(f) == family(lid)]
                if chosen:
                    picked = lid
                    break
            else:
                chosen = [f for f in files if named(f) is None]
        for f in chosen:
            out.append((label(f), read(os.path.join(pkg_dir, f))))

        # A license file can be a pointer ("dual licensed, see the links") or an SPDX
        # document rather than the license: the canonical text makes up for it
        found = " ".join(" ".join(text for _, text in out).split())

        def holds(lid):
            return lid not in FINGERPRINT or FINGERPRINT[lid] in found

        known = [lid for lid in sorted(ids, key=rank) if lid in FINGERPRINT]
        if all_of:
            need = [lid for lid in ids if not holds(lid)]
        elif picked:
            need = [] if holds(picked) else [picked]
        elif chosen and (len(known) < len(ids) or any(holds(lid) for lid in known)):
            need = []  # a file under a license with no fingerprint is taken at its word
        else:
            need = [next((lid for lid in known if canonical(lid, holders)), known[0])] if known else []
        for lid in need:
            text = canonical(lid, holders)
            if text is None:
                errors.append(f"{pkg}: none of its files holds the {lid} text, and scripts/licenses/ has none either")
            else:
                out.append((lid, text))
        if not out:
            errors.append(f"{pkg}: ships no license file, and none of {ids} has a text in scripts/licenses/")
    for f in notice:
        out.append(("NOTICE", read(os.path.join(pkg_dir, f))))
    for rel in vendored:
        path = os.path.join(pkg_dir, rel)
        if os.path.isfile(path):
            out.append((f"{named(os.path.basename(rel)) or 'LICENSE'} ({os.path.dirname(rel)})", read(path)))
        else:
            errors.append(f"{pkg}: VENDORED names {rel}, which is not in the package")
    return out


def holders_of(names, fallback):
    names = [re.sub(r"\s*<[^>]*>", "", n).strip() for n in names]
    names = [n for n in names if n]
    return ", ".join(names) if names else f"the {fallback} authors"


def crates():
    """(name, version) → package, for every crate linked into the app on a release target."""
    shipped = {}
    for target in TARGETS:
        meta = json.loads(subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", target],
            cwd=ROOT, check=True, capture_output=True, text=True).stdout)
        packages = {p["id"]: p for p in meta["packages"]}
        nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
        workspace = set(meta["workspace_members"])
        stack = [i for i in workspace if os.path.samefile(packages[i]["manifest_path"], APP_MANIFEST)]
        seen = set()
        while stack:
            i = stack.pop()
            if i in seen:
                continue
            seen.add(i)
            stack += [d["pkg"] for d in nodes[i]["deps"] if any(k["kind"] is None for k in d["dep_kinds"])]
        for i in seen - workspace:
            p = packages[i]
            if any("proc-macro" in t["kind"] for t in p["targets"]):
                continue
            shipped[(p["name"], p["version"])] = p
    return shipped


def resolve(name, start):
    """Node's lookup: the nearest node_modules/<name> above `start` (works for pnpm's layout too)."""
    d = start
    while True:
        if os.path.basename(d) != "node_modules":
            candidate = os.path.join(d, "node_modules", name)
            if os.path.isfile(os.path.join(candidate, "package.json")):
                return os.path.realpath(candidate)
        parent = os.path.dirname(d)
        if parent == d:
            return None
        d = parent


def npm_packages():
    """Real path → package.json, for every package the frontend bundle can draw on."""
    with open(os.path.join(ROOT, "package.json"), encoding="utf-8") as f:
        root = json.load(f)
    queue = [(name, ROOT) for name in [*root.get("dependencies", {}), "svelte"]]
    seen = {}
    while queue:
        name, base = queue.pop()
        d = resolve(name, base)
        if d is None:
            errors.append(f"{name} is not installed under {base} (run pnpm install)")
            continue
        if d in seen:
            continue
        with open(os.path.join(d, "package.json"), encoding="utf-8") as f:
            meta = json.load(f)
        seen[d] = meta
        optional = set(meta.get("optionalDependencies", {}))
        queue += [(dep, d) for dep in meta.get("dependencies", {}) if dep not in optional]
    return seen


def npm_license(meta):
    lic = meta.get("license")
    if isinstance(lic, dict):
        lic = lic.get("type")
    if not lic and isinstance(meta.get("licenses"), list):
        lic = " OR ".join(x.get("type", "") if isinstance(x, dict) else str(x) for x in meta["licenses"])
    return lic


groups = {}  # text → {"labels": set, "packages": set}


def add(entries, line):
    for label, text in entries:
        g = groups.setdefault(text, {"labels": set(), "packages": set()})
        g["labels"].add(label)
        g["packages"].add(line)


def heading(g):
    """One text can be filed under `MIT` by one package and `MIT OR Apache-2.0` by
    another; the shortest label is the one that says what the text is."""
    return min(g["labels"], key=lambda label: (len(label), label))


rust = crates()
for (name, version), p in sorted(rust.items()):
    pkg_dir = os.path.dirname(p["manifest_path"])
    holders = holders_of(p.get("authors") or [], name)
    add(notices(f"{name} {version}", pkg_dir, p.get("license"), p.get("license_file"), holders, VENDORED.get(name, ())),
        (name, version, f"https://crates.io/crates/{name}/{version}"))

npm = npm_packages()
for d, meta in sorted(npm.items(), key=lambda kv: (kv[1]["name"], kv[1]["version"])):
    name, version = meta["name"], meta["version"]
    author = meta.get("author")
    author = author.get("name", "") if isinstance(author, dict) else (author or "")
    add(notices(f"{name} {version}", d, npm_license(meta), None, holders_of([author], name)),
        (name, version, f"https://www.npmjs.com/package/{name}/v/{version}"))

if errors:
    for e in errors:
        print("error:", e)
    raise SystemExit(1)

RULE = "=" * 80
parts = [
    "THIRD-PARTY SOFTWARE NOTICES",
    "",
    "vemry is built from the third-party packages listed below. Their licenses ask",
    "for these notices to go with every copy of vemry. Each package's source code is",
    "available at the address given next to it.",
    "",
    "vemry itself is licensed under FSL-1.1-ALv2; see LICENSE.",
    "",
    "This file is generated by scripts/gen-notices.py from Cargo.lock and",
    "pnpm-lock.yaml. Do not edit it by hand.",
    "",
    f"{len(rust)} Rust crates, {len(npm)} npm packages, {len(groups)} distinct license texts.",
]

for text, g in sorted(groups.items(), key=lambda kv: (heading(kv[1]).lower(), min(kv[1]["packages"]))):
    pkgs = sorted(g["packages"])
    width = max(len(f"{n} {v}") for n, v, _ in pkgs)
    parts += ["", RULE, heading(g), ""]
    parts += [f"  {f'{n} {v}':<{width}}  {url}" for n, v, url in pkgs]
    parts += ["", text]

with open(OUT, "w", encoding="utf-8", newline="\n") as f:
    f.write("\n".join(parts) + "\n")
print(f"wrote {OUT}  ({len(rust)} crates, {len(npm)} npm packages, {len(groups)} texts)")
