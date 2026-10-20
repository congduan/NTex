#!/usr/bin/env python3
"""校验默认宏文件、锁定导入清单与实际 Tauri bundle；可从已校验 vendor 树恢复资产。"""
import argparse
import csv
import hashlib
import json
import struct
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ASSETS = REPO / "assets" / "tex-minimal"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def bundle_entries(path):
    data = path.read_bytes()
    if data[:8] != b"NTEXBND1":
        raise ValueError("bundle 魔数不匹配")
    count, = struct.unpack_from("<I", data, 8)
    pos = 12
    result = {}
    for _ in range(count):
        kind = data[pos]
        name_len, = struct.unpack_from("<I", data, pos + 1)
        pos += 5
        name = data[pos:pos + name_len].decode("utf-8")
        pos += name_len
        size, = struct.unpack_from("<I", data, pos)
        pos += 4
        if pos + size > len(data):
            raise ValueError(f"bundle 条目截断：{name}")
        if (kind, name) in result:
            raise ValueError(f"bundle 重复键：{kind}/{name}")
        result[kind, name] = digest(data[pos:pos + size])
        pos += size
    if pos != len(data):
        raise ValueError("bundle 有未解析尾部")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, help="额外检查实际 Tauri 资产包")
    parser.add_argument("--restore-from", type=Path, help="从 ntex-pkg vendor 暂存树恢复上游资产")
    args = parser.parse_args()
    with (ASSETS / "runtime-assets.tsv").open() as stream:
        rows = list(csv.DictReader(stream, delimiter="\t"))
    problems = []
    pending = []
    for row in rows:
        relative = Path(row["# path"])
        if relative.is_absolute() or ".." in relative.parts or relative.parts[0] != "tex":
            raise ValueError(f"清单路径不合法：{relative}")
        target = ASSETS / relative
        if row["policy"] not in {"upstream", "preserved"}:
            raise ValueError(f"未知导入策略：{row['policy']}")
        if args.restore_from:
            source = args.restore_from / relative
            if not source.is_file() or digest(source.read_bytes()) != row["source_sha256"]:
                problems.append(f"源文件缺失或校验不匹配：{relative}")
                continue
            if row["policy"] == "upstream":
                pending.append((target, source))
                continue
        if not target.is_file() or digest(target.read_bytes()) != row["shipped_sha256"]:
            problems.append(f"发行文件缺失或发生漂移：{relative}")
    if problems:
        raise ValueError("\n".join(problems))
    # 全部校验成功才写入；preserved 项保留原格式所用的基础宏文件。
    for target, source in pending:
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(source.read_bytes())
    config = json.loads((ASSETS / "default-packages.json").read_text())
    required = [name + ".sty" for name in config["packages"]]
    required += [name + ".cls" for name in config["documentclasses"]]
    files = {}
    for path in (ASSETS / "tex").rglob("*"):
        if path.is_file():
            files.setdefault(path.name, set()).add(digest(path.read_bytes()))
    for name in required:
        if name not in files:
            raise ValueError(f"默认入口缺失：{name}")
    if args.bundle:
        entries = bundle_entries(args.bundle)
        for name, hashes in files.items():
            if entries.get((0, name)) not in hashes:
                raise ValueError(f"实际 bundle 缺文件或内容漂移：{name}")
        print(f"实际 bundle：{len(files)} 个宏文件主名全部在包内")
    preserved = sum(row["policy"] == "preserved" for row in rows)
    print(f"PASS：{len(required)} 个默认入口，{len(rows)} 个校验文件，{preserved} 个基础版本保留项")


if __name__ == "__main__":
    main()
