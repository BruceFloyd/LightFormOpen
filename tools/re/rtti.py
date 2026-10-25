"""Recover C++ class inheritance from Itanium RTTI in an x86_64 Mach-O dylib.
Usage: rtti.py DYLIB FIXUPS_TXT > edges.tsv   (child \t base \t kind)"""
import re
import struct
import subprocess
import sys

path, fixups = sys.argv[1], sys.argv[2]
buf = open(path, 'rb').read()

# segments: vmaddr -> fileoff
segs = []
ncmds = struct.unpack_from('<I', buf, 16)[0]
off = 32
for _ in range(ncmds):
    cmd, size = struct.unpack_from('<II', buf, off)
    if cmd == 0x19:
        vmaddr, vmsize, fileoff, filesize = struct.unpack_from('<QQQQ', buf, off + 24)
        segs.append((vmaddr, vmsize, fileoff, filesize))
    off += size

def rd(addr):
    for va, vs, fo, fs in segs:
        if va <= addr < va + fs:
            return struct.unpack_from('<Q', buf, fo + addr - va)[0]
    return None

def cstr(addr):
    for va, vs, fo, fs in segs:
        if va <= addr < va + fs:
            p = fo + addr - va
            return buf[p:buf.index(b'\0', p)].decode('latin1')
    return None

binds = {}
for line in open(fixups):
    m = re.match(r'\S+\s+\S+\s+(0x[0-9A-F]+)\s+pointer\s+-?\d+\s+\S+\s+(\S+)', line)
    if m:
        binds[int(m.group(1), 16)] = m.group(2)

# local typeinfo symbols (also non-exported ones: use nm without -g)
ti = {}
for line in subprocess.run(['nm', path], capture_output=True, text=True).stdout.splitlines():
    parts = line.split()
    if len(parts) == 3 and parts[2].startswith('__ZTI'):
        ti[int(parts[0], 16)] = parts[2]

def demangle(names):
    out = subprocess.run(['c++filt'], input='\n'.join(names), capture_output=True, text=True).stdout
    return out.splitlines()

def name_of(addr):
    if addr in ti:
        return ti[addr]
    return None

edges = []
for addr, sym in ti.items():
    vt = binds.get(addr)
    if vt is None:
        continue
    if '__si_class_type_info' in vt:
        base_ptr = addr + 16
        base = binds.get(base_ptr) or name_of(rd(base_ptr))
        edges.append((sym, base, 'si'))
    elif '__vmi_class_type_info' in vt:
        cnt = struct.unpack_from('<I', buf, 0)[0]
        # read count via segment mapping
        for va, vs, fo, fs in segs:
            if va <= addr + 20 < va + fs:
                cnt = struct.unpack_from('<I', buf, fo + addr + 20 - va)[0]
        for i in range(min(cnt, 32)):
            bp = addr + 24 + i * 16
            base = binds.get(bp) or name_of(rd(bp))
            flags = rd(bp + 8) or 0
            edges.append((sym, base, 'virtual' if flags & 1 else 'mi'))
    elif '__class_type_info' in vt:
        edges.append((sym, None, 'root'))

dm = {}
allnames = sorted({e[0] for e in edges} | {e[1] for e in edges if e[1]})
for n, d in zip(allnames, demangle(allnames)):
    dm[n] = d.replace('typeinfo for ', '')
for c, b, k in edges:
    print(f'{dm[c]}\t{dm.get(b, b) if b else "-"}\t{k}')
