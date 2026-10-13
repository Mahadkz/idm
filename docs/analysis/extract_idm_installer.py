"""Read-only extraction of the supplied IDM installer's record payload.

Recovered proprietary files belong in a private analysis directory.
This does not execute the installer or alter the source file.
"""
import argparse
import hashlib
import json
import re
import struct
import zlib
from pathlib import Path
import pefile

parser = argparse.ArgumentParser()
parser.add_argument('installer', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
if args.output.exists() and any(args.output.iterdir()):
    raise SystemExit('Use an empty output directory to preserve previous evidence.')
data = args.installer.read_bytes()
pe = pefile.PE(data=data)
position = max(s.PointerToRawData + s.SizeOfRawData for s in pe.sections)
archive_start = position
records, payloads = [], []
while position + 32 <= len(data):
    words = struct.unpack_from('<8I', data, position)
    compressed_size = words[1]
    if compressed_size == 0 or position + 32 + compressed_size > len(data):
        break
    compressed = data[position+32:position+32+compressed_size]
    inflater = zlib.decompressobj()
    try:
        payload = inflater.decompress(compressed, 64 * 1024 * 1024)
    except zlib.error:
        break
    if not inflater.eof or inflater.unused_data or inflater.unconsumed_tail:
        raise ValueError(f'Unexpected stream bounds at {position}')
    payloads.append(payload)
    records.append({'index': len(records), 'header_offset': position,
                    'compressed_bytes': compressed_size, 'output_bytes': len(payload),
                    'header_words': words, 'sha256': hashlib.sha256(payload).hexdigest()})
    position += 32 + compressed_size
if not payloads or not payloads[0].startswith(b'<DEST>'):
    raise SystemExit('This installer does not have the recovered target structure.')
manifest = payloads[0].decode('utf-8')
names = {int(i): name for i, name in re.findall(r'<P(\d+)="([^"]+)"', manifest)}
if set(names) != set(range(1, len(payloads))):
    raise ValueError('Manifest and payload records do not match completely.')
args.output.mkdir(parents=True, exist_ok=True)
(args.output / 'installer_payload_manifest.txt').write_text(manifest)
files = args.output / 'files'
for i, name in names.items():
    relative = Path(name.replace('\\', '/'))
    if relative.is_absolute() or '..' in relative.parts or ':' in name:
        raise ValueError(f'Unsafe payload path: {name}')
    destination = files / relative
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(payloads[i])
    records[i]['original_name'] = name
records[0]['original_name'] = 'installer_payload_manifest'
inventory = {'installer_sha256': hashlib.sha256(data).hexdigest(),
             'archive_start': archive_start, 'archive_end': position,
             'tail_bytes': len(data)-position, 'records': records}
(args.output / 'inventory.json').write_text(json.dumps(inventory, indent=2) + '\n')
print(json.dumps({'records': len(records), 'named_files': len(names),
                  'archive_start': archive_start, 'archive_end': position}))
