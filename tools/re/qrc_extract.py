"""Recover Qt rcc resource trees embedded in a Mach-O binary.

Strategy: find name-table entries by verifying qt_hash, find tree nodes that
point at those names, then locate the data blob by aligning a file node's
data_offset with a length-prefixed payload. Usage: qrc_extract.py BINARY OUTDIR
"""
import os
import re
import struct
import sys
import zlib

def qt_hash(s):
    h = 0
    for ch in s:
        h = ((h << 4) + ord(ch)) & 0xFFFFFFFF
        h ^= (h & 0xF0000000) >> 23
        h &= 0x0FFFFFFF
    return h

def find_names(buf):
    """Return {abs_offset: name} for every hash-verified rcc name entry."""
    names = {}
    # names are UTF-16BE; look for runs of ASCII-in-UTF16 of length >= 1
    for m in re.finditer(rb'(?:\x00[\x20-\x7e]){1,200}', buf):
        start = m.start()
        # entry header (u16 len, u32 hash) sits 6 bytes before the chars;
        # the run may start mid-name, so try each char boundary in the run
        for s in range(start, min(m.end(), start + 2), 2):
            if s < 6:
                continue
            ln, h = struct.unpack_from('>HI', buf, s - 6)
            if ln == 0 or s + 2 * ln > len(buf):
                continue
            try:
                name = buf[s:s + 2 * ln].decode('utf-16-be')
            except UnicodeDecodeError:
                continue
            if qt_hash(name) == h:
                names[s - 6] = name
    return names

def main():
    path, out = sys.argv[1], sys.argv[2]
    buf = open(path, 'rb').read()
    import pickle
    cache = os.path.join(os.path.dirname(os.path.abspath(__file__)), os.path.basename(path) + '.names.pkl')
    if os.path.exists(cache):
        names = pickle.load(open(cache, 'rb'))
    else:
        names = find_names(buf)
        pickle.dump(names, open(cache, 'wb'))
    if not names:
        sys.exit('no rcc names found')
    # group names into contiguous blobs; each blob is one names table
    offs = sorted(names)
    blobs, cur = [], [offs[0]]
    for o in offs[1:]:
        prev = cur[-1]
        if 0 <= o - (prev + 6 + 2 * len(names[prev])) <= 512:
            cur.append(o)
        else:
            blobs.append(cur)
            cur = [o]
    blobs.append(cur)
    blobs = [b for b in blobs if len(b) >= 3]
    total = 0
    used_trees = set()
    for bi, blob in enumerate(blobs):
        nbase = blob[0]
        rel = {o - nbase: names[o] for o in blob}
        # search for a tree: root dir with first child 1 whose whole walk
        # resolves every name inside this names table
        def full_walk_ok(t, nsz):
            seen, miss, stack = 0, 0, [0]
            while stack:
                i = stack.pop()
                seen += 1
                if seen > 20000 or t + (i + 1) * nsz > len(buf):
                    return False
                no, fl = struct.unpack_from('>IH', buf, t + i * nsz)
                if i and no not in rel:
                    miss += 1
                    if miss > 2 + seen // 10:
                        return False
                if fl & 2:
                    c, f = struct.unpack_from('>II', buf, t + i * nsz + 6)
                    if c > 5000 or (c and f <= i):
                        return False
                    stack.extend(range(f, f + c))
                elif fl & ~7:
                    return False
            return seen if seen > 1 and miss * 10 <= seen else 0
        best = None
        for node_size in (22, 14):
            for m in re.finditer(rb'\x00\x00\x00\x00\x00\x02', buf):
                t = m.start()
                if t in used_trees:
                    continue
                cnt, first = struct.unpack_from('>II', buf, t + 6)
                if cnt == 0 or cnt > 10000 or first != 1:
                    continue
                size = full_walk_ok(t, node_size)
                if size and (not best or size > best[2]):
                    best = (t, node_size, size)
            if best:
                break
        if not best:
            print(f'blob {bi}: {len(blob)} names, no tree')
            continue
        used_trees.add(best[0])
        tree, ns = best[0], best[1]

        def node(i):
            no, flags = struct.unpack_from('>IH', buf, tree + i * ns)
            if flags & 2:
                cnt, first = struct.unpack_from('>II', buf, tree + i * ns + 6)
                return rel.get(no, f'unnamed_{i}'), flags, cnt, first
            _, _, doff = struct.unpack_from('>HHI', buf, tree + i * ns + 6)
            return rel.get(no, f'unnamed_{i}'), flags, doff, None

        files = []
        def walk(i, prefix):
            name, flags, a, b = node(i)
            p = f'{prefix}/{name}' if name and i else prefix
            if flags & 2:
                for c in range(b, b + a):
                    walk(c, p)
            else:
                files.append((p, flags, a))
        walk(0, '')
        # align data blob: every uncompressed .qml payload starting with
        # 'import' yields a candidate base; keep the one most files agree with
        def score(base):
            good = 0
            for p, flags, doff in files:
                at = base + doff
                if at < 0 or at + 8 > len(buf):
                    return -1
                ln = struct.unpack_from('>I', buf, at)[0]
                if ln > 64_000_000 or at + 4 + ln > len(buf):
                    continue
                if flags & 1:
                    good += buf[at + 8] == 0x78
                elif flags & 4:
                    good += buf[at + 4:at + 8] == b'\x28\xb5\x2f\xfd'
                else:
                    good += 1
            return good
        sigs = {'.qml': rb'import Qt', '.png': rb'\x89PNG\r\n', '.svg': rb'<(?:\?xml|svg)', '.json': rb'[{\[]'}
        hits = {ext: [m.start() for m in re.finditer(rx, buf)] for ext, rx in sigs.items() if ext != '.json'}
        cands = set()
        for p, flags, doff in files[:40]:
            ext = os.path.splitext(p)[1]
            if flags & 5 or ext not in hits:
                continue
            for pos in hits[ext]:
                ln = struct.unpack_from('>I', buf, pos - 4)[0]
                if 0 < ln < 2_000_000:
                    cands.add(pos - 4 - doff)
        data_base, best_score = None, 0
        for c in cands:
            sc = score(c)
            if sc > best_score:
                data_base, best_score = c, sc
        print(f'blob {bi}: best alignment {best_score}/{len(files)}')
        if data_base is None:
            print(f'blob {bi}: tree found ({len(files)} files) but no data alignment')
            continue
        for p, flags, doff in files:
            ln = struct.unpack_from('>I', buf, data_base + doff)[0]
            raw = buf[data_base + doff + 4:data_base + doff + 4 + ln]
            if flags & 1:
                try:
                    raw = zlib.decompress(raw[4:])
                except zlib.error:
                    print(f'  zlib fail {p} len={ln} head={raw[:12].hex()}')
                    p += '.bad'
            elif flags & 4:
                p += '.zst'
            dest = os.path.join(out, f'tree{bi}', p.lstrip('/'))
            try:
                os.makedirs(os.path.dirname(dest), exist_ok=True)
            except FileExistsError:
                print(f'  path collision {p}')
                continue
            if os.path.isdir(dest):
                dest += '.file'
            with open(dest, 'wb') as fh:
                fh.write(raw)
            total += 1
        print(f'blob {bi}: {len(files)} files, tree@{tree:#x} data@{data_base:#x}')
    print(f'extracted {total} files')

main()
