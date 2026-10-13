import json
import argparse
import struct
from pathlib import Path
import pefile

parser = argparse.ArgumentParser(description='Inspect recovered IDM PE resources without executing them.')
parser.add_argument('extraction_directory', type=Path)
BASE = parser.parse_args().extraction_directory

class Reader:
    def __init__(self, data):
        self.data, self.pos = data, 0
    def take(self, fmt):
        result = struct.unpack_from('<' + fmt, self.data, self.pos)
        self.pos += struct.calcsize('<' + fmt)
        return result[0] if len(result) == 1 else result
    def align(self):
        self.pos = (self.pos + 3) & ~3
    def text(self):
        first = self.take('H')
        if first == 0xffff:
            return {'ordinal': self.take('H')}
        words = []
        while first:
            words.append(first)
            first = self.take('H')
        return struct.pack('<' + 'H' * len(words), *words).decode('utf-16-le')

def dialog(data):
    r = Reader(data)
    extended = data[:4] == b'\x01\x00\xff\xff'
    if extended:
        version, signature, help_id, ex_style, style, count, x, y, cx, cy = r.take('HHIIIHhhhh')
    else:
        style, ex_style, count, x, y, cx, cy = r.take('IIHhhhh')
    result = {'extended': extended, 'style': style, 'ex_style': ex_style,
              'bounds_dlu': [x, y, cx, cy], 'menu': r.text(), 'class': r.text(),
              'title': r.text(), 'controls': []}
    if style & 0x40:
        font = {'size': r.take('H')}
        if extended:
            font.update(zip(['weight', 'italic', 'charset'], r.take('HBB')))
        font['face'] = r.text()
        result['font'] = font
    for _ in range(count):
        r.align()
        if extended:
            help_id, ex_style, style, x, y, cx, cy, cid = r.take('IIIhhhhI')
        else:
            style, ex_style, x, y, cx, cy, cid = r.take('IIhhhhH')
        control = {'id': cid, 'style': style, 'ex_style': ex_style,
                   'bounds_dlu': [x, y, cx, cy], 'class': r.text(), 'title': r.text()}
        extra = r.take('H')
        if extra:
            r.pos += extra - 2
        result['controls'].append(control)
    assert r.pos <= len(data)
    return result

def menu(data):
    r = Reader(data)
    version, offset = r.take('HH')
    if version != 0:
        return {'version': version, 'unparsed': True}
    r.pos = 4 + offset
    def level():
        entries = []
        while True:
            flags = r.take('H')
            popup = bool(flags & 0x10)
            cid = None if popup else r.take('H')
            item = {'id': cid, 'flags': flags, 'text': r.text()}
            if popup:
                item['children'] = level()
            entries.append(item)
            if flags & 0x80:
                return entries
    return {'version': version, 'entries': level()}

def inspect(name):
    path = BASE / 'files' / name
    pe = pefile.PE(str(path))
    output = {'file': name, 'machine': pe.FILE_HEADER.Machine,
              'image_base': pe.OPTIONAL_HEADER.ImageBase,
              'entrypoint_rva': pe.OPTIONAL_HEADER.AddressOfEntryPoint,
              'dialogs': [], 'menus': [], 'strings': [], 'errors': [],
              'imports': {entry.dll.decode(): [imp.name.decode() if imp.name else imp.ordinal
                                              for imp in entry.imports]
                          for entry in getattr(pe, 'DIRECTORY_ENTRY_IMPORT', [])}}
    for typ in getattr(pe, 'DIRECTORY_ENTRY_RESOURCE', []).entries:
        if typ.id not in [4, 5, 6]:
            continue
        for entry in typ.directory.entries:
            for lang in entry.directory.entries:
                raw = pe.get_data(lang.data.struct.OffsetToData, lang.data.struct.Size)
                identity = {'resource_id': entry.id, 'language': lang.id,
                            'data_rva': lang.data.struct.OffsetToData,
                            'data_file_offset': pe.get_offset_from_rva(lang.data.struct.OffsetToData)}
                try:
                    if typ.id == 5:
                        output['dialogs'].append({**identity, **dialog(raw)})
                    elif typ.id == 4:
                        output['menus'].append({**identity, **menu(raw)})
                    else:
                        r = Reader(raw)
                        for i in range(16):
                            length = r.take('H')
                            text = raw[r.pos:r.pos+length*2].decode('utf-16-le')
                            r.pos += length * 2
                            if text:
                                output['strings'].append({**identity, 'string_id': (entry.id-1)*16+i,
                                                          'text': text})
                except Exception as error:
                    output['errors'].append({**identity, 'type': typ.id, 'error': str(error)})
    return output

if __name__ == '__main__':
    for name in ['IDMan.exe', 'IDMGrHlp.exe', 'IDMMsgHost.exe']:
        output = inspect(name)
        target = BASE / (name + '.resources.json')
        target.write_text(json.dumps(output, indent=2, ensure_ascii=False))
        print(name, 'dialogs', len(output['dialogs']), 'menus', len(output['menus']),
              'strings', len(output['strings']), 'errors', output['errors'])
        if name == 'IDMan.exe':
            for d in output['dialogs']:
                labels = [c['title'] for c in d['controls'] if isinstance(c['title'], str) and c['title']]
                print(d['resource_id'], repr(d['title']), len(d['controls']), repr(labels[:5]))
