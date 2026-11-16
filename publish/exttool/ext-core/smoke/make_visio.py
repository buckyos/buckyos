"""Writes a two-page Visio 2013 drawing (VSDX, the Open Packaging format Visio saves since 2013), built from the
published schema (MS-VSDX) so it can be redistributed: the soffice smoke converts it, and nfs_server's Preview tests
keep the copy it wrote as tests/fixtures/preview/drawing.vsdx.

Page 1 "流程 Flow": three boxes with Chinese and Latin text, two connectors with arrow heads glued to them, and a group
(a framed box with a label inside). Page 2 "图片 Picture": an embedded PNG picture and a caption.

    python3 make_visio.py <output.vsdx>
"""

import struct
import sys
import zlib
import zipfile
from pathlib import Path

NS = 'xmlns="http://schemas.microsoft.com/office/visio/2012/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xml:space="preserve"'
REL = "http://schemas.microsoft.com/visio/2010/relationships"
HEAD = '<?xml version="1.0" encoding="utf-8" standalone="yes"?>\n'


def png(width, height, colour):
    """A solid-colour RGB PNG with a darker frame."""
    frame = tuple(c // 2 for c in colour)
    rows = b"".join(b"\0" + b"".join(bytes(frame if x in (0, width - 1) or y in (0, height - 1) else colour) for x in range(width)) for y in range(height))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")


def cells(**values):
    return "".join(f'<Cell N="{name}" V="{value}"/>' for name, value in values.items())


def rect(w, h):
    return ('<Section N="Geometry" IX="0"><Row T="MoveTo" IX="1"><Cell N="X" V="0"/><Cell N="Y" V="0"/></Row>'
            f'<Row T="LineTo" IX="2"><Cell N="X" V="{w}"/><Cell N="Y" V="0"/></Row><Row T="LineTo" IX="3"><Cell N="X" V="{w}"/><Cell N="Y" V="{h}"/></Row>'
            f'<Row T="LineTo" IX="4"><Cell N="X" V="0"/><Cell N="Y" V="{h}"/></Row><Row T="LineTo" IX="5"><Cell N="X" V="0"/><Cell N="Y" V="0"/></Row></Section>')


def char(size=0.1667, colour="#000000"):
    return f'<Section N="Character"><Row IX="0"><Cell N="Color" V="{colour}"/><Cell N="Size" V="{size}" U="PT"/></Row></Section>'


def box(id, name, x, y, w, h, text, fill):
    return (f'<Shape ID="{id}" NameU="{name}" Name="{name}" Type="Shape">'
            + cells(PinX=x, PinY=y, Width=w, Height=h, LocPinX=w / 2, LocPinY=h / 2, Angle=0, FillForegnd=fill, FillPattern=1, LineWeight=0.02, LineColor="#1f3864", LinePattern=1)
            + char() + rect(w, h) + f"<Text>{text}</Text></Shape>")


def connector(id, x1, y1, x2, y2):
    """A straight 1-D connector with an arrow head at its end."""
    import math
    length = math.hypot(x2 - x1, y2 - y1)
    angle = math.atan2(y2 - y1, x2 - x1)
    return (f'<Shape ID="{id}" NameU="Dynamic connector.{id}" Name="Dynamic connector.{id}" Type="Shape">'
            + cells(PinX=(x1 + x2) / 2, PinY=(y1 + y2) / 2, Width=length, Height=0, LocPinX=length / 2, LocPinY=0, Angle=angle,
                    BeginX=x1, BeginY=y1, EndX=x2, EndY=y2, LineWeight=0.02, LineColor="#c00000", LinePattern=1, EndArrow=13, EndArrowSize=2, ObjType=2)
            + f'<Section N="Geometry" IX="0"><Cell N="NoFill" V="1"/><Row T="MoveTo" IX="1"><Cell N="X" V="0"/><Cell N="Y" V="0"/></Row>'
            + f'<Row T="LineTo" IX="2"><Cell N="X" V="{length}"/><Cell N="Y" V="0"/></Row></Section></Shape>')


def group(id, x, y, w, h):
    """A group: a filled frame and a labelled box inside it, in the group's own coordinates."""
    return (f'<Shape ID="{id}" NameU="Group" Name="Group" Type="Group">'
            + cells(PinX=x, PinY=y, Width=w, Height=h, LocPinX=w / 2, LocPinY=h / 2, Angle=0)
            + "<Shapes>"
            + f'<Shape ID="{id + 1}" NameU="Frame" Name="Frame" Type="Shape">' + cells(PinX=w / 2, PinY=h / 2, Width=w, Height=h, LocPinX=w / 2, LocPinY=h / 2, FillForegnd="#e2efda", FillPattern=1, LineWeight=0.01, LineColor="#548235", LinePattern=2) + rect(w, h) + "</Shape>"
            + box(id + 2, "Inner", w / 2, h / 2, w * 0.7, h * 0.45, "组合 Group", "#ffffff")
            + "</Shapes></Shape>")


def picture(id, x, y, w, h):
    return (f'<Shape ID="{id}" NameU="Picture" Name="Picture" Type="Foreign">'
            + cells(PinX=x, PinY=y, Width=w, Height=h, LocPinX=w / 2, LocPinY=h / 2, Angle=0, ImgOffsetX=0, ImgOffsetY=0, ImgWidth=w, ImgHeight=h, LinePattern=0)
            + rect(w, h) + '<ForeignData ForeignType="Bitmap" CompressionType="PNG"><Rel r:id="rId1"/></ForeignData></Shape>')


def page_sheet(width, height):
    return '<PageSheet LineStyle="0" FillStyle="0" TextStyle="0">' + cells(PageWidth=width, PageHeight=height, PageScale=1, DrawingScale=1, DrawingSizeType=0, PrintPageOrientation=2) + "</PageSheet>"


def build():
    flow = ('<PageContents ' + NS + '><Shapes>'
            + box(1, "Start", 1.5, 6.5, 1.8, 0.8, "开始 Start", "#dae3f3")
            + box(2, "Check", 5.0, 6.5, 2.2, 0.8, "检查 Check", "#fff2cc")
            + box(3, "Done", 8.5, 6.5, 1.8, 0.8, "完成 Done", "#dae3f3")
            + connector(4, 2.4, 6.5, 3.9, 6.5)
            + connector(5, 6.1, 6.5, 7.6, 6.5)
            + group(6, 5.0, 3.0, 3.0, 1.6)
            + "</Shapes><Connects>"
            + '<Connect FromSheet="4" FromCell="BeginX" FromPart="9" ToSheet="1" ToCell="PinX" ToPart="3"/>'
            + '<Connect FromSheet="4" FromCell="EndX" FromPart="12" ToSheet="2" ToCell="PinX" ToPart="3"/>'
            + '<Connect FromSheet="5" FromCell="BeginX" FromPart="9" ToSheet="2" ToCell="PinX" ToPart="3"/>'
            + '<Connect FromSheet="5" FromCell="EndX" FromPart="12" ToSheet="3" ToCell="PinX" ToPart="3"/>'
            + "</Connects></PageContents>")
    gallery = ('<PageContents ' + NS + '><Shapes>'
               + picture(1, 5.5, 4.75, 4.0, 3.0)
               + box(2, "Caption", 5.5, 2.4, 4.0, 0.6, "图片说明 Picture caption", "#ffffff")
               + "</Shapes></PageContents>")
    document = ('<VisioDocument ' + NS + '><DocumentSettings TopPage="0" DefaultTextStyle="0" DefaultLineStyle="0" DefaultFillStyle="0" DefaultGuideStyle="0"/>'
                '<Colors><ColorEntry IX="0" RGB="#000000"/><ColorEntry IX="1" RGB="#FFFFFF"/></Colors>'
                '<FaceNames><FaceName NameU="Calibri" UnicodeRanges="-536859905 -1073732485 9 0" CharSets="536871423 0" Panose="2 15 5 2 2 2 4 3 2 4" Flags="325"/></FaceNames>'
                '<StyleSheets><StyleSheet ID="0" NameU="No Style" Name="No Style">'
                + cells(EnableLineProps=1, EnableFillProps=1, EnableTextProps=1, LineWeight=0.01, LineColor=0, LinePattern=1, FillForegnd=1, FillPattern=1)
                + '<Section N="Character"><Row IX="0"><Cell N="Font" V="Calibri"/><Cell N="Color" V="0"/><Cell N="Size" V="0.1666666666666667"/></Row></Section>'
                + '</StyleSheet></StyleSheets></VisioDocument>')
    pages = ('<Pages ' + NS + '>'
             + f'<Page ID="0" NameU="Flow" Name="流程 Flow" ViewScale="1" ViewCenterX="5.5" ViewCenterY="4.25">{page_sheet(11, 8.5)}<Rel r:id="rId1"/></Page>'
             + f'<Page ID="4" NameU="Picture" Name="图片 Picture" ViewScale="1" ViewCenterX="5.5" ViewCenterY="4.25">{page_sheet(11, 8.5)}<Rel r:id="rId2"/></Page>'
             + "</Pages>")
    parts = {
        "[Content_Types].xml": '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/>'
        '<Override PartName="/visio/document.xml" ContentType="application/vnd.ms-visio.drawing.main+xml"/><Override PartName="/visio/pages/pages.xml" ContentType="application/vnd.ms-visio.pages+xml"/>'
        '<Override PartName="/visio/pages/page1.xml" ContentType="application/vnd.ms-visio.page+xml"/><Override PartName="/visio/pages/page2.xml" ContentType="application/vnd.ms-visio.page+xml"/>'
        '<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/></Types>',
        "_rels/.rels": f'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/document" Target="visio/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>',
        "docProps/core.xml": '<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Preview sample 预览样例</dc:title><dc:creator>BuckyOS tests</dc:creator></cp:coreProperties>',
        "visio/document.xml": document,
        "visio/_rels/document.xml.rels": f'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/pages" Target="pages/pages.xml"/></Relationships>',
        "visio/pages/pages.xml": pages,
        "visio/pages/_rels/pages.xml.rels": f'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{REL}/page" Target="page1.xml"/><Relationship Id="rId2" Type="{REL}/page" Target="page2.xml"/></Relationships>',
        "visio/pages/page1.xml": flow,
        "visio/pages/page2.xml": gallery,
        "visio/pages/_rels/page2.xml.rels": '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/></Relationships>',
    }
    out = Path(sys.argv[1])
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for name, text in parts.items():
            info = zipfile.ZipInfo(name, date_time=(2026, 11, 16, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(info, HEAD + text)
        info = zipfile.ZipInfo("visio/media/image1.png", date_time=(2026, 11, 16, 0, 0, 0))
        z.writestr(info, png(96, 72, (68, 114, 196)))


if __name__ == "__main__":
    build()
