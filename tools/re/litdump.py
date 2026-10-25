"""Read a Lightform Creator .lit project (binary or JSON container) and dump
it as normalized JSON. Also reads .litsettings (same container).

Timeline time/duration values are ticks at 705,600,000 per second (flicks).
Binary v23 projects elide static Property objects, so an object's references
point straight at value controllers; JSON v9 projects keep the Property hop.

Usage: python3 -I litdump.py PROJECT.lit [--registry class_registry.json] [--tree]

Output (default): {"container", "header", "objects": {uuid: {...}}}
--tree additionally nests objects under their owners for reading.
"""
import datetime
import json
import re
import struct
import sys
import uuid

# Chunk-type UUIDs (named in libLightformEngine's string table next to each id)
CHUNK = {
    '8e73ed04-344f-45f0-8821-85fa77dcb9b5': 'header',
    '4505e8fb-03b9-45ae-bb0d-94c83bc2944d': 'id-lookup',
    '54fa5fd0-d3cb-472f-b660-3bb67f89eef4': 'objects',
    '787ea4ea-18e1-4e76-97d4-2795d6fe8f50': 'object',
    '0c5a8d0c-d533-47b7-bc25-4e21443714e1': 'state',
    '95623259-f9de-4a79-922b-e4eef38436dd': 'references',
    '607eb63c-c47e-4a87-97b3-135429656984': 'reference',
    'cb30171e-c4bc-4682-901d-1e2cbc802498': 'properties',
    '15a78dd1-1e93-44aa-af02-f02085d75711': 'property',
    '6ce5a4ab-d5c1-4d94-ac26-0fe82546a1cf': 'dynamic-description',
}

# Property keys are stored as a 32-bit hash; these are every key name found in
# the engine's chunk-property-name table.
KEY_NAMES = [
    'magic', 'format', 'format-version', 'max-bytes-per-line', 'id', 'name',
    'class-id', 'class-name', 'control-type', 'count', 'data-version-number',
    'default-value', 'enum-description-id', 'flags', 'group-name',
    'interface-description-id', 'is-array', 'local-id', 'maximum-value',
    'minimum-value', 'object-description-id', 'original-data-version-number',
    'owner-id', 'precision', 'purpose-enum-description-id',
    'purpose-enum-value', 'range', 'subtype-id', 'value',
    'value-controller-object-description-id', 'value-count', 'value-names',
    'value-type', 'values',
]


def key_hash(s):
    h = 0
    for c in s.encode():
        h = (h * 33 + c) & 0xFFFFFFFF
    return h


KEYS = {key_hash(k): k for k in KEY_NAMES}

# Element codecs for the 'values' array, keyed by value-controller type suffix.
# 'f32' etc. are per-component; the array's leading u32 counts components.
SCALAR = {
    'plain-float': 'f', 'plain-vector2': 'f', 'plain-vector3': 'f',
    'plain-vector4': 'f', 'plain-color4f': 'f', 'plain-matrix3': 'f',
    'plain-matrix4': 'f', 'plain-double': 'd', 'plain-angle': 'd',
    'plain-int64': 'q', 'plain-int32': 'i', 'plain-vector2i': 'i',
    'plain-vector3i': 'i', 'plain-vector4i': 'i', 'plain-quaternion': 'f',
    'plain-distance': 'd', 'plain-color4u8': 'B', 'plain-timeline-time': 'q',
    'plain-timeline-duration': 'q', 'plain-date-time': 'q', 'plain-boolean': '?',
}
STRINGISH = {'plain-string', 'plain-url', 'plain-enum', 'plain-file-system-path'}


# --- binary container -------------------------------------------------------

def parse_chunks(b, i=0):
    """Return ([node], next_index). node = ('obj', uuid, [node]) or
    (tag, key_name, payload_bytes). Tag 2 closes an object, tag 3 ends the
    document."""
    nodes = []
    while i < len(b):
        t = b[i]
        if t in (2, 3):
            return nodes, i + 1
        ln = struct.unpack_from('<I', b, i + 1)[0]
        p = b[i + 5:i + 5 + ln]
        i += 5 + ln
        if t == 1:
            kids, i = parse_chunks(b, i)
            u = str(uuid.UUID(bytes=p))
            nodes.append(('obj', CHUNK.get(u, u), kids))
        else:
            k = struct.unpack_from('<I', p)[0]
            nodes.append((t, KEYS.get(k, f'#{k:08x}'), p[4:]))
    return nodes, i


def props(nodes):
    return {n[1]: (n[0], n[2]) for n in nodes if n[0] != 'obj'}


def subobjs(nodes, name):
    return [n for n in nodes if n[0] == 'obj' and n[1] == name]


def u32(p):
    return struct.unpack_from('<I', p)[0]


def lpstr(p, off=0):
    n = u32(p[off:])
    return p[off + 4:off + 4 + n].decode('utf-8', 'replace'), off + 4 + n


def decode_values(raw, ctype):
    """Decode a tag-5 'values' payload using the owning controller's type."""
    n = u32(raw)
    body = raw[4:]
    suffix = (ctype or '').rsplit('.', 1)[-1]
    if suffix in SCALAR:
        fmt = SCALAR[suffix]
        vals = list(struct.unpack_from(f'<{n}{fmt}', body))
        if suffix == 'plain-date-time':  # Windows FILETIME (100 ns since 1601)
            return [datetime.datetime.fromtimestamp(v / 1e7 - 11644473600, datetime.timezone.utc)
                    .strftime('%Y-%m-%dT%H:%M:%SZ') for v in vals]
        return vals
    if suffix in STRINGISH:
        out, off = [], 0
        for _ in range(n):
            s, off = lpstr(body, off)
            out.append(s)
        return out
    if suffix == 'plain-uuid':
        return [str(uuid.UUID(bytes=body[16 * i:16 * i + 16])) for i in range(n)]
    return {'count': n, 'raw': body.hex()}


def scalar(tag, p, name):
    # length-prefixed UTF-8 string (u32 byte count + bytes)
    if name not in ('id', 'class-id', 'owner-id', 'local-id', 'count') and len(p) >= 4 \
            and u32(p) == len(p) - 4 and len(p) > 4:
        return p[4:].decode('utf-8', 'replace')
    if name in ('id', 'class-id', 'owner-id', 'local-id', 'format-version',
                'data-version-number', 'original-data-version-number',
                'precision', 'flags'):
        return u32(p) if len(p) == 4 else struct.unpack_from('<q', p)[0]
    if name in ('count',):
        return struct.unpack_from('<q', p)[0] if len(p) == 8 else u32(p)
    if name == 'is-array':
        return bool(p[0])
    if name in ('name', 'format', 'class-name', 'group-name', 'subtype-id'):
        if len(p) >= 4 and u32(p) == len(p) - 4:
            return p[4:].decode('utf-8', 'replace')
    if name == 'magic':
        return p.decode('latin-1')
    if name == 'max-bytes-per-line':
        return struct.unpack_from('<q', p)[0]
    return {'tag': tag, 'hex': p.hex()}


def read_binary(b, registry):
    top, _ = parse_chunks(b)
    hdr = {k: scalar(t, p, k) for k, (t, p) in props(top).items()}
    for h in subobjs(top, 'header'):
        hdr.update({k: scalar(t, p, k) for k, (t, p) in props(h[2]).items()})
    look = subobjs(top, 'id-lookup')[0][2]
    raw = props(look)['values'][1]
    ids = [str(uuid.UUID(bytes=raw[4 + 16 * i:20 + 16 * i])) for i in range(u32(raw))]

    def ref(p):
        return ids[u32(p)]

    objects = {}
    for o in subobjs(top, 'objects')[0][2]:
        if o[0] != 'obj':
            continue
        pr = props(o[2])
        oid = ref(pr['id'][1])
        cid = ref(pr['class-id'][1])
        rec = {'name': None, 'classId': cid,
               'ownerId': ref(pr['owner-id'][1]) if 'owner-id' in pr else None}
        if 'name' in pr:
            rec['name'] = lpstr(pr['name'][1])[0]
        ctype = registry.get(cid, {}).get('type')
        for st in subobjs(o[2], 'state'):
            rec['state'] = state = {}
            for k, (t, p) in props(st[2]).items():
                if k == 'values' and t == 5:
                    state['values'] = decode_values(p, ctype)
                elif k == 'subtype-id' and len(p) == 4:
                    state['subtypeId'] = ref(p)
                else:
                    state[camel(k)] = scalar(t, p, k)
            for dd in subobjs(st[2], 'dynamic-description'):
                state['dynamicDescription'] = read_dyndesc(dd[2], ids)
        for rs in subobjs(o[2], 'references'):
            refs = rec['references'] = {}
            for r in subobjs(rs[2], 'reference'):
                rp = props(r[2])
                refs[str(u32(rp['local-id'][1]))] = ref(rp['id'][1])
        objects[oid] = rec
    return hdr, objects


def read_dyndesc(nodes, ids):
    out = {}
    for k, (t, p) in props(nodes).items():
        if k in ('subtype-id', 'enum-description-id', 'object-description-id',
                 'value-controller-object-description-id',
                 'purpose-enum-description-id', 'interface-description-id') and len(p) == 4:
            out[camel(k)] = ids[u32(p)]
        elif k == 'name':
            out['name'] = lpstr(p)[0]
        elif t == 5:
            out[camel(k)] = {'count': u32(p), 'hex': p[4:].hex()}
        else:
            v = scalar(t, p, k)
            out[camel(k)] = v
    return out


def camel(k):
    a, *rest = k.split('-')
    return a + ''.join(x.title() for x in rest)


# --- JSON container ---------------------------------------------------------

def read_json(text):
    # The writer emits trailing commas; strip them before parsing.
    d = json.loads(re.sub(r',(\s*[}\]])', r'\1', text))
    hdr = {k: v for k, v in d.items() if '|' not in k}
    objects = {}
    for k, v in d.items():
        base = k.split('|')[0]
        if base == 'header':
            hdr.update(v)
        elif base == 'objects':
            for idx, o in v.items():
                if idx == 'count':
                    continue
                rec = {'name': o.get('name'), 'classId': o['classId'],
                       'ownerId': o.get('ownerId')}
                for kk, vv in o.items():
                    kb = kk.split('|')[0]
                    if kb == 'state':
                        st = dict(vv)
                        for sk in list(st):
                            if sk.startswith('dynamicDescription|'):
                                st['dynamicDescription'] = st.pop(sk)
                        vals = st.get('values')
                        if isinstance(vals, list):  # timeline ticks are "<n>flk"
                            st['values'] = [int(x[:-3]) if isinstance(x, str) and x.endswith('flk')
                                            and x[:-3].lstrip('-').isdigit() else x for x in vals]
                        rec['state'] = st
                    elif kb == 'references':
                        rec['references'] = {
                            str(r['localId']): r['id']
                            for rk, r in vv.items() if rk != 'count'}
                objects[o['id']] = rec
    return hdr, objects


# --- entry point --------------------------------------------------------------

def load(path, registry=None):
    registry = registry or {}
    b = open(path, 'rb').read()
    if b[:1] == b'{':
        container = 'json'
        hdr, objects = read_json(b.decode('utf-8'))
    else:
        container = 'binary'
        hdr, objects = read_binary(b, registry)
    for rec in objects.values():
        info = registry.get(rec['classId'])
        if info:
            rec['class'] = info['type']
    return {'container': container, 'header': hdr, 'objects': objects}


def nest(doc):
    objs = doc['objects']
    kids = {}
    for oid, o in objs.items():
        kids.setdefault(o.get('ownerId'), []).append(oid)

    def build(oid):
        o = dict(objs[oid], id=oid)
        ch = [build(c) for c in kids.get(oid, [])]
        if ch:
            o['children'] = ch
        return o
    roots = [oid for oid, o in objs.items() if o.get('ownerId') not in objs]
    return [build(r) for r in roots]


if __name__ == '__main__':
    args = sys.argv[1:]
    reg = {}
    if '--registry' in args:
        i = args.index('--registry')
        reg = json.load(open(args[i + 1]))
        del args[i:i + 2]
    tree = '--tree' in args
    args = [a for a in args if a != '--tree']
    doc = load(args[0], reg)
    if tree:
        doc = {'container': doc['container'], 'header': doc['header'],
               'tree': nest(doc)}
    json.dump(doc, sys.stdout, indent=1)
