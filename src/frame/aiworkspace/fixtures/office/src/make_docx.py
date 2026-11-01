"""docx fixtures for the Office importer (Office 导入 TODO O0), made with python-docx plus raw WordprocessingML where
python-docx has no API: a title in the core properties, the Title style, headings 1–4, runs bold / italic / struck,
a hyperlink, nested bullets, a numbered list and one restarting at 5, Quote and a Code paragraph style, a run of empty
paragraphs, a line break and a tab, an inline picture after text, a table with a repeated header row, a horizontal
and a vertical merge, a page break; then revisions (w:ins / w:del), a comment, a footnote and an endnote, simple and
complex fields (PAGE, HYPERLINK, a TOC with its cached entries), a block content control, a text box, a formula.

Run from this folder: python3 make_docx.py   (writes ../docx/basic.docx, ../docx/features.docx; needs python-docx and
Pillow — on Debian `apt install python3-docx python3-pil`)"""

import io

from docx import Document
from docx.enum.style import WD_STYLE_TYPE
from docx.enum.text import WD_BREAK
from docx.opc.constants import RELATIONSHIP_TYPE as RT
from docx.opc.packuri import PackURI
from docx.opc.part import Part
from docx.oxml import parse_xml
from docx.oxml.ns import nsdecls, qn
from docx.shared import Cm, Pt
from PIL import Image, ImageDraw

W = nsdecls('w')
MC = ('xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" '
      'xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" xmlns:v="urn:schemas-microsoft-com:vml"')


def picture(w, h, color):
    img = Image.new('RGB', (w, h), color)
    d = ImageDraw.Draw(img)
    d.rectangle([w // 8, h // 8, w - w // 8, h - h // 8], outline=(255, 255, 255), width=4)
    d.line([0, 0, w, h], fill=(255, 255, 255), width=3)
    out = io.BytesIO()
    img.save(out, 'PNG')
    out.seek(0)
    return out


def hyperlink(paragraph, url, text):
    r_id = paragraph.part.relate_to(url, RT.HYPERLINK, is_external=True)
    link = parse_xml(f'<w:hyperlink {W} {nsdecls("r")} r:id="{r_id}"><w:r><w:rPr><w:rStyle w:val="Hyperlink"/></w:rPr><w:t>{text}</w:t></w:r></w:hyperlink>')
    paragraph._p.append(link)


def basic():
    doc = Document()
    doc.core_properties.title = '季度报告 Quarterly Report'
    code = doc.styles.add_style('Code', WD_STYLE_TYPE.PARAGRAPH)
    code.base_style = doc.styles['Normal']
    code.font.name = 'Courier New'
    code.font.size = Pt(9)

    doc.add_paragraph('Quarterly Report', style='Title')
    doc.add_heading('Overview', level=1)
    p = doc.add_paragraph('Plain, ')
    p.add_run('bold').bold = True
    p.add_run(', ')
    p.add_run('italic').italic = True
    p.add_run(', ')
    p.add_run('struck').font.strike = True
    p.add_run(', ')
    both = p.add_run('bold italic')
    both.bold = True
    both.italic = True
    p.add_run(' and a link to ')
    hyperlink(p, 'https://example.com/report', 'example.com')
    p.add_run('.')
    doc.add_heading('Details', level=2)
    doc.add_heading('Fine print', level=3)
    doc.add_heading('Finer print', level=4)

    doc.add_paragraph('First point', style='List Bullet')
    doc.add_paragraph('Second point', style='List Bullet')
    doc.add_paragraph('A sub-point', style='List Bullet 2')
    doc.add_paragraph('Third point', style='List Bullet')
    doc.add_paragraph('Step one', style='List Number')
    doc.add_paragraph('Step two', style='List Number')
    doc.add_paragraph('Step three', style='List Number')

    doc.add_paragraph('Simplicity is prerequisite for reliability.', style='Quote')
    doc.add_paragraph('fn main() {', style='Code')
    doc.add_paragraph('    println!("hi");', style='Code')
    doc.add_paragraph('}', style='Code')

    for _ in range(3):
        doc.add_paragraph('')
    br = doc.add_paragraph('Line one')
    br.runs[0].add_break()
    br.add_run('line two\tafter a tab')

    fig = doc.add_paragraph('Figure 1: ')
    fig.add_run().add_picture(picture(400, 200, (37, 99, 235)), width=Cm(8))
    fig.add_run(' (blue)')

    table = doc.add_table(rows=4, cols=3)
    table.style = 'Table Grid'
    for k, h in enumerate(['Region', 'Q1', 'Q2']):
        table.cell(0, k).text = h
    table.rows[0]._tr.get_or_add_trPr().append(parse_xml(f'<w:tblHeader {W}/>'))
    rows = [['North', '12', '15'], ['South', '9', '11'], ['Total', '21', '26']]
    for r, row in enumerate(rows, start=1):
        for k, v in enumerate(row):
            table.cell(r, k).text = v
    table.cell(1, 0).merge(table.cell(2, 0)).text = 'North and South'
    table.cell(3, 1).merge(table.cell(3, 2))

    doc.add_paragraph('Before the break').add_run().add_break(WD_BREAK.PAGE)
    zh = doc.add_paragraph('中文段落，包含')
    zh.add_run('粗体').bold = True
    zh.add_run('与普通文字。')
    doc.save('../docx/basic.docx')


def add_part(doc, name, content_type, xml, reltype):
    part = Part(PackURI(name), content_type, xml.encode('utf-8'), doc.part.package)
    return doc.part.relate_to(part, reltype)


def features():
    doc = Document()
    doc.core_properties.title = ''
    body = doc.element.body
    sect = body[-1]

    def raw(xml):
        sect.addprevious(parse_xml(xml))

    doc.add_heading('Features', level=1)
    # revisions: an insertion kept, a deletion dropped
    raw(f'<w:p {W}><w:r><w:t xml:space="preserve">Kept </w:t></w:r><w:ins w:id="1" w:author="a" w:date="2026-10-01T00:00:00Z"><w:r><w:t xml:space="preserve">inserted </w:t></w:r></w:ins><w:del w:id="2" w:author="a" w:date="2026-10-01T00:00:00Z"><w:r><w:delText xml:space="preserve">deleted </w:delText></w:r></w:del><w:r><w:t>text.</w:t></w:r></w:p>')
    # a comment
    add_part(doc, '/word/comments.xml', 'application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml',
             f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:comments {W}><w:comment w:id="0" w:author="a" w:date="2026-10-01T00:00:00Z"><w:p><w:r><w:t>A remark</w:t></w:r></w:p></w:comment></w:comments>',
             'http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments')
    raw(f'<w:p {W}><w:commentRangeStart w:id="0"/><w:r><w:t>Commented words</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:commentReference w:id="0"/></w:r></w:p>')
    # a footnote and an endnote
    note = lambda tag, kind: (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:{tag}s {W}>'
                              f'<w:{tag} w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:{tag}>'
                              f'<w:{tag} w:type="continuationSeparator" w:id="0"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:{tag}>'
                              f'<w:{tag} w:id="1"><w:p><w:r><w:{kind}Ref/></w:r><w:r><w:t xml:space="preserve"> The {tag} text.</w:t></w:r></w:p></w:{tag}></w:{tag}s>')
    add_part(doc, '/word/footnotes.xml', 'application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml', note('footnote', 'footnote'),
             'http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes')
    add_part(doc, '/word/endnotes.xml', 'application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml', note('endnote', 'endnote'),
             'http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes')
    raw(f'<w:p {W}><w:r><w:t>With a footnote</w:t></w:r><w:r><w:rPr><w:vertAlign w:val="superscript"/></w:rPr><w:footnoteReference w:id="1"/></w:r><w:r><w:t xml:space="preserve"> and an endnote</w:t></w:r><w:r><w:endnoteReference w:id="1"/></w:r><w:r><w:t>.</w:t></w:r></w:p>')
    # fields: PAGE (simple), HYPERLINK (complex), a TOC with cached entries
    raw(f'<w:p {W}><w:r><w:t xml:space="preserve">Page </w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p>')
    raw(f'<w:p {W}><w:r><w:t xml:space="preserve">Visit </w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> HYPERLINK "https://buckyos.ai/" </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>BuckyOS</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:t>.</w:t></w:r></w:p>')
    raw(f'<w:p {W}><w:pPr><w:pStyle w:val="TOC1"/></w:pPr><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> TOC \\o "1-3" \\h \\z \\u </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>Features 1</w:t></w:r></w:p>')
    raw(f'<w:p {W}><w:pPr><w:pStyle w:val="TOC1"/></w:pPr><w:r><w:t>Numbers 2</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>')
    # a block content control
    raw(f'<w:sdt {W}><w:sdtPr><w:alias w:val="Box"/></w:sdtPr><w:sdtContent><w:p><w:r><w:t>Inside a content control</w:t></w:r></w:p></w:sdtContent></w:sdt>')
    # a text box (DrawingML with its VML fallback)
    raw(f'''<w:p {W} {nsdecls("wp", "a")} {MC}><w:r><w:t xml:space="preserve">Anchor </w:t></w:r><w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><wp:anchor distT="0" distB="0" distL="0" distR="0" simplePos="0" relativeHeight="1" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="1828800" cy="457200"/><wp:wrapSquare wrapText="bothSides"/><wp:docPr id="10" name="Text Box 10"/><wp:cNvGraphicFramePr/><a:graphic><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:cNvSpPr txBox="1"/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1828800" cy="457200"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></wps:spPr><wps:txbx><w:txbxContent><w:p><w:r><w:t>Floating box text</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></mc:Choice><mc:Fallback><w:pict><v:shape id="tb10" type="#_x0000_t202" style="width:144pt;height:36pt"><v:textbox><w:txbxContent><w:p><w:r><w:t>Floating box text</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></mc:Fallback></mc:AlternateContent></w:r></w:p>''')
    # a formula
    raw(f'<w:p {W} {nsdecls("m")}><w:r><w:t xml:space="preserve">Energy: </w:t></w:r><m:oMath><m:r><m:t>E=m</m:t></m:r><m:sSup><m:e><m:r><m:t>c</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup></m:oMath></w:p>')
    # a numbered list restarting at 5 (a numbering override), and a deeper level
    doc.add_paragraph('Five', style='List Number')
    doc.add_paragraph('Six', style='List Number')
    numbering = doc.part.numbering_part.element
    abstract = numbering.num_having_numId(doc.styles['List Number'].element.pPr.numPr.numId.val).abstractNumId.val
    num = numbering.add_num(abstract)
    num.add_lvlOverride(0).add_startOverride(5)
    for p in doc.paragraphs[-2:]:
        p._p.get_or_add_pPr().get_or_add_numPr().get_or_add_numId().val = num.numId
        p._p.pPr.numPr.get_or_add_ilvl().val = 0
    # a table with a vertical merge written by hand (restart / continue)
    raw(f'<w:tbl {W}><w:tblPr><w:tblStyle w:val="TableGrid"/></w:tblPr><w:tblGrid><w:gridCol w:w="2000"/><w:gridCol w:w="2000"/></w:tblGrid>'
        f'<w:tr><w:tc><w:tcPr><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>Tall</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>a</w:t></w:r></w:p><w:p><w:r><w:t>b</w:t></w:r></w:p></w:tc></w:tr>'
        f'<w:tr><w:tc><w:tcPr><w:vMerge/></w:tcPr><w:p/></w:tc><w:tc><w:tbl><w:tblGrid><w:gridCol w:w="1000"/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>nested</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p/></w:tc></w:tr></w:tbl>')
    raw(f'<w:p {W}><w:r><w:rPr><w:vanish/></w:rPr><w:t>hidden text</w:t></w:r><w:r><w:t>Visible end.</w:t></w:r></w:p>')
    doc.save('../docx/features.docx')


basic()
features()
