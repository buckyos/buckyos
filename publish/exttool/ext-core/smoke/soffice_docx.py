from io import BytesIO
from pathlib import Path
import sys
from xml.dom import minidom
from zipfile import ZipFile


WORD = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
FONT = "Noto Sans CJK SC"


def set_fonts(fonts):
    for name in ("asciiTheme", "hAnsiTheme", "eastAsiaTheme", "cstheme"):
        if fonts.hasAttributeNS(WORD, name):
            fonts.removeAttributeNS(WORD, name)
    for name in ("ascii", "hAnsi", "eastAsia", "cs"):
        fonts.setAttributeNS(WORD, f"w:{name}", FONT)


def child(parent, name):
    for node in parent.childNodes:
        if node.namespaceURI == WORD and node.localName == name:
            return node
    node = parent.ownerDocument.createElementNS(WORD, f"w:{name}")
    parent.insertBefore(node, parent.firstChild)
    return node


def prepare(path):
    output = BytesIO()
    with ZipFile(path) as source, ZipFile(output, "w") as target:
        target.comment = source.comment
        for entry in source.infolist():
            data = source.read(entry)
            if entry.filename in ("word/document.xml", "word/styles.xml"):
                document = minidom.parseString(data)
                if entry.filename == "word/document.xml":
                    for run in document.getElementsByTagNameNS(WORD, "r"):
                        if run.getElementsByTagNameNS(WORD, "t"):
                            set_fonts(child(child(run, "rPr"), "rFonts"))
                else:
                    for fonts in document.getElementsByTagNameNS(WORD, "rFonts"):
                        set_fonts(fonts)
                data = document.toxml(encoding="UTF-8")
                document.unlink()
            target.writestr(entry, data)
    path.write_bytes(output.getvalue())


if __name__ == "__main__":
    prepare(Path(sys.argv[1]))
