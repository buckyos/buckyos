"""xlsx fixtures for the table importer (表格数据导入 TODO T4), written part by part (no openpyxl): shared strings
with rich runs and phonetic guides, inline strings, cached formula values, 1900 and 1904 date systems, built-in and
custom number formats (date, date-time, money `[$¥-804]`, percentage, time of day), a merged header, an error cell,
a hidden sheet, two visible sheets.

Run from this folder: python3 make_xlsx.py   (writes ../xlsx/*.xlsx)"""

import datetime
import zipfile

def serial(y, m, d, h=0, mi=0, s=0, base1904=False):
    epoch = datetime.datetime(1904, 1, 1) if base1904 else datetime.datetime(1899, 12, 30)
    delta = datetime.datetime(y, m, d, h, mi, s) - epoch
    return delta.days + delta.seconds / 86400

CONTENT_TYPES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>{sheets}<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/></Types>"""
ROOT_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"""
NS = 'xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"'

# cell styles: 0 general, 1 m/d/yyyy (14), 2 money [$¥-804] (164), 3 0.00% (10), 4 m/d/yyyy h:mm (22), 5 h:mm (20), 6 yyyy"年"m"月"d"日" (165)
STYLES = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><numFmts count="2"><numFmt numFmtId="164" formatCode="[$¥-804]#,##0.00"/><numFmt numFmtId="165" formatCode="yyyy&quot;年&quot;m&quot;月&quot;d&quot;日&quot;"/></numFmts><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts><fills count="1"><fill><patternFill patternType="none"/></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="7"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="14" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="10" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="22" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="20" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="165" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/></cellXfs></styleSheet>"""

class Strings:
    def __init__(self):
        self.items = []
    def add(self, xml_si):
        self.items.append(xml_si)
        return len(self.items) - 1
    def text(self, t):
        return self.add(f'<si><t>{t}</t></si>')
    def xml(self):
        return f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="{len(self.items)}" uniqueCount="{len(self.items)}">' + ''.join(self.items) + '</sst>'

def sheet(rows, merges=()):
    out = []
    for r, cells in rows:
        out.append(f'<row r="{r}">' + ''.join(cells) + '</row>')
    merge = f'<mergeCells count="{len(merges)}">' + ''.join(f'<mergeCell ref="{m}"/>' for m in merges) + '</mergeCells>' if merges else ''
    return f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<worksheet {NS}><dimension ref="A1:G6"/><sheetData>' + ''.join(out) + '</sheetData>' + merge + '</worksheet>'

def s(ref, i): return f'<c r="{ref}" t="s"><v>{i}</v></c>'
def n(ref, v, style=0): return f'<c r="{ref}"{f" s=\"{style}\"" if style else ""}><v>{v}</v></c>'
def f(ref, formula, v, style=0): return f'<c r="{ref}"{f" s=\"{style}\"" if style else ""}><f>{formula}</f><v>{v}</v></c>'
def b(ref, v): return f'<c r="{ref}" t="b"><v>{1 if v else 0}</v></c>'
def e(ref, v): return f'<c r="{ref}" t="e"><v>{v}</v></c>'
def inline(ref, t): return f'<c r="{ref}" t="inlineStr"><is><t>{t}</t></is></c>'

def workbook(path, sheets, strings, date1904=False):
    names = ''.join(f'<sheet name="{name}" sheetId="{i + 1}"{f" state=\"{state}\"" if state else ""} r:id="rId{i + 1}"/>' for i, (name, state, _) in enumerate(sheets))
    wb = f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<workbook {NS}><workbookPr{" date1904=\"1\"" if date1904 else ""}/><sheets>{names}</sheets></workbook>'
    rels = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' + ''.join(
        f'<Relationship Id="rId{i + 1}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{i + 1}.xml"/>' for i in range(len(sheets))) + \
        f'<Relationship Id="rId{len(sheets) + 1}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId{len(sheets) + 2}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/></Relationships>'
    overrides = ''.join(f'<Override PartName="/xl/worksheets/sheet{i + 1}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>' for i in range(len(sheets)))
    with zipfile.ZipFile(path, 'w', zipfile.ZIP_DEFLATED) as z:
        for name, data in [('[Content_Types].xml', CONTENT_TYPES.format(sheets=overrides)), ('_rels/.rels', ROOT_RELS), ('xl/workbook.xml', wb), ('xl/_rels/workbook.xml.rels', rels),
                           ('xl/styles.xml', STYLES), ('xl/sharedStrings.xml', strings.xml())] + [(f'xl/worksheets/sheet{i + 1}.xml', x) for i, (_, _, x) in enumerate(sheets)]:
            info = zipfile.ZipInfo(name, date_time=(2026, 10, 30, 0, 0, 0))
            # the first part stored, the others deflated: both methods are read
            info.compress_type = zipfile.ZIP_STORED if name == '[Content_Types].xml' else zipfile.ZIP_DEFLATED
            z.writestr(info, data)

# TI-09: two visible sheets and a hidden one
st = Strings()
month, revenue, rate, total, note, paid = (st.text(x) for x in ['Month', 'Revenue', 'Rate', 'Total', 'Note', 'Paid'])
rich = st.add('<si><r><rPr><b/></rPr><t>Big</t></r><r><t xml:space="preserve"> deal</t></r></si>')
ruby = st.add('<si><t>東京</t><rPh sb="0" eb="2"><t>トウキョウ</t></rPh><phoneticPr fontId="1"/></si>')
plain = st.text('ok')
sales = sheet([
    (1, [s('A1', month), s('B1', revenue), s('D1', rate), s('E1', total), s('F1', note), s('G1', paid)]),
    (2, [n('A2', serial(2026, 1, 1), 1), n('B2', 1200.5, 2), n('C2', 1, 0), n('D2', 0.125, 3), f('E2', 'B2*D2', 150.0625), s('F2', rich), b('G2', True)]),
    (3, [n('A3', serial(2026, 2, 1), 1), n('B3', 980, 2), n('C3', 2, 0), n('D3', 0.05, 3), f('E3', 'B3*D3', 49), s('F3', ruby), b('G3', False)]),
    (4, [n('A4', serial(2026, 3, 1), 1), n('B4', 15000.75, 2), n('C4', 3, 0), n('D4', 0.2, 3), e('E4', '#DIV/0!'), inline('F4', 'inline &amp; text'), b('G4', True)]),
], merges=['B1:C1'])
events = sheet([
    (1, [s('A1', st.text('Event')), s('B1', st.text('At')), s('C1', st.text('Time')), s('D1', st.text('Day'))]),
    (2, [s('A2', st.text('Kickoff')), n('B2', serial(2026, 7, 1, 9, 30), 4), n('C2', 0.5, 5), n('D2', serial(2026, 7, 1), 6)]),
    (3, [s('A3', plain), n('B3', serial(2026, 7, 2, 18, 0, 15), 4), n('C3', 0.75, 5), n('D3', serial(2026, 7, 2), 6)]),
])
hidden = sheet([(1, [s('A1', st.text('secret'))]), (2, [n('A2', 1)])])
workbook('../xlsx/workbook.xlsx', [('Sales', None, sales), ('Events', None, events), ('Hidden', 'hidden', hidden)], st)

# the 1904 date system (old Mac workbooks)
st = Strings()
mac = sheet([
    (1, [s('A1', st.text('Day')), s('B1', st.text('Count'))]),
    (2, [n('A2', serial(2026, 7, 1, base1904=True), 1), n('B2', 3)]),
    (3, [n('A3', serial(2026, 7, 2, base1904=True), 1), n('B3', 4)]),
])
workbook('../xlsx/dates1904.xlsx', [('Mac', None, mac)], st, date1904=True)
print('wrote ../xlsx/workbook.xlsx ../xlsx/dates1904.xlsx')
