"""Find serialized FileDescriptorProto blobs in a binary and render .proto text.
Usage: proto_extract.py BINARY OUTDIR [skip-prefix ...]"""
import os
import re
import sys

def varint(b, i):
    r = s = 0
    while True:
        if i >= len(b):
            raise ValueError
        c = b[i]; i += 1
        r |= (c & 0x7F) << s; s += 7
        if not c & 0x80:
            return r, i
        if s > 63:
            raise ValueError

def fields(b):
    i, out = 0, []
    while i < len(b):
        k, i = varint(b, i)
        f, wt = k >> 3, k & 7
        if wt == 0:
            v, i = varint(b, i)
        elif wt == 2:
            n, i = varint(b, i)
            if i + n > len(b):
                raise ValueError
            v = b[i:i + n]; i += n
        elif wt == 1:
            v = b[i:i + 8]; i += 8
        elif wt == 5:
            v = b[i:i + 4]; i += 4
        else:
            raise ValueError
        out.append((f, wt, v))
    return out

# top-level FileDescriptorProto fields and their wire types
FD_WT = {1: 2, 2: 2, 3: 2, 4: 2, 5: 2, 6: 2, 7: 2, 8: 2, 9: 2, 10: 0, 11: 0, 12: 2, 14: 0}

def blob_len(b, start):
    """Greedily consume top-level fields that are valid for FileDescriptorProto."""
    i = start
    last = start
    while i < len(b):
        try:
            k, j = varint(b, i)
        except ValueError:
            break
        f, wt = k >> 3, k & 7
        if FD_WT.get(f) != wt:
            break
        try:
            if wt == 0:
                _, j = varint(b, j)
            else:
                n, j = varint(b, j)
                if n > 1_000_000 or j + n > len(b):
                    break
                j += n
        except ValueError:
            break
        i = last = j
    return last - start

TYPES = {1: 'double', 2: 'float', 3: 'int64', 4: 'uint64', 5: 'int32', 6: 'fixed64',
         7: 'fixed32', 8: 'bool', 9: 'string', 12: 'bytes', 13: 'uint32', 15: 'sfixed32',
         16: 'sfixed64', 17: 'sint32', 18: 'sint64'}
LABEL = {1: '', 2: 'required ', 3: 'repeated '}

def s(v):
    return v.decode('utf-8', 'replace')

def render_enum(b, ind):
    fs = fields(b)
    name = next(s(v) for f, _, v in fs if f == 1)
    out = [f'{ind}enum {name} {{']
    for f, _, v in fs:
        if f == 2:
            ev = dict((ff, vv) for ff, _, vv in fields(v))
            num = ev.get(2, 0)
            if num >= 1 << 63:
                num -= 1 << 64
            out.append(f'{ind}  {s(ev[1])} = {num};')
    out.append(f'{ind}}}')
    return out

def render_msg(b, ind, syntax3):
    fs = fields(b)
    name = next(s(v) for f, _, v in fs if f == 1)
    out = [f'{ind}message {name} {{']
    oneofs = [s(dict((ff, vv) for ff, _, vv in fields(v))[1]) for f, _, v in fs if f == 8]
    groups = {}
    plain = []
    for f, _, v in fs:
        if f == 2:
            fd = {}
            for ff, _, vv in fields(v):
                fd[ff] = vv
            t = TYPES.get(fd.get(5)) or s(fd.get(6, b'?'))
            if t.startswith('.'):
                t = t[1:]
            lab = LABEL.get(fd.get(4, 1), '')
            if fd.get(17):
                lab = 'optional '
            line = f'{s(fd[1])} = {fd[3]};'
            if 9 in fd and not fd.get(17):
                groups.setdefault(fd[9], []).append(f'{t} {line}')
            else:
                plain.append(f'{ind}  {lab}{t} {line}')
    out += plain
    for idx, lines in groups.items():
        out.append(f'{ind}  oneof {oneofs[idx]} {{')
        out += [f'{ind}    {l}' for l in lines]
        out.append(f'{ind}  }}')
    for f, _, v in fs:
        if f == 3:
            out += render_msg(v, ind + '  ', syntax3)
        elif f == 4:
            out += render_enum(v, ind + '  ')
    out.append(f'{ind}}}')
    return out

def render_service(b):
    fs = fields(b)
    name = next(s(v) for f, _, v in fs if f == 1)
    out = [f'service {name} {{']
    for f, _, v in fs:
        if f == 2:
            md = {}
            for ff, _, vv in fields(v):
                md[ff] = vv
            cs = 'stream ' if md.get(5) else ''
            ss = 'stream ' if md.get(6) else ''
            out.append(f'  rpc {s(md[1])}({cs}{s(md[2]).lstrip(".")}) returns ({ss}{s(md[3]).lstrip(".")});')
    out.append('}')
    return out

def render(b):
    fs = fields(b)
    d = {}
    for f, _, v in fs:
        d.setdefault(f, []).append(v)
    syntax = s(d[12][0]) if 12 in d else 'proto2'
    out = [f'// recovered from embedded FileDescriptorProto', f'syntax = "{syntax}";']
    if 2 in d:
        out.append(f'package {s(d[2][0])};')
    for dep in d.get(3, []):
        out.append(f'import "{s(dep)}";')
    out.append('')
    for e in d.get(5, []):
        out += render_enum(e, '') + ['']
    for m in d.get(4, []):
        out += render_msg(m, '', syntax == 'proto3') + ['']
    for sv in d.get(6, []):
        out += render_service(sv) + ['']
    return s(d[1][0]), '\n'.join(out)

def main():
    path, outdir, skip = sys.argv[1], sys.argv[2], tuple(sys.argv[3:])
    buf = open(path, 'rb').read()
    seen = set()
    for m in re.finditer(rb'\x0a([\x01-\x7f])([A-Za-z0-9_./-]+\.proto)', buf):
        if m.group(1)[0] != len(m.group(2)):
            continue
        fname = m.group(2).decode()
        if fname in seen or fname.startswith(skip):
            continue
        n = blob_len(buf, m.start())
        if n < 20:
            continue
        try:
            name, text = render(buf[m.start():m.start() + n])
        except Exception as e:
            print('fail', fname, e)
            continue
        seen.add(fname)
        dest = os.path.join(outdir, name)
        os.makedirs(os.path.dirname(dest) or outdir, exist_ok=True)
        open(dest, 'w').write(text)
        print(f'{fname}\t{n} bytes @ {m.start():#x}')

main()
