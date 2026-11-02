#!/bin/sh
# LibreOffice-saved copies of the generated fixtures (Office 导入 TODO O0): the same documents as LibreOffice writes
# OOXML, and the ODF versions for the faithful (LibreOffice → PDF) path. Needs soffice. Run from this folder.
set -e
profile=$(mktemp -d)
lo() { soffice "-env:UserInstallation=file://$profile" --headless --norestore --nologo "$@" >/dev/null 2>&1; }
tmp=$(mktemp -d)
lo --convert-to 'docx:MS Word 2007 XML' --outdir "$tmp" ../docx/basic.docx
mv "$tmp/basic.docx" ../docx/basic.lo.docx
lo --convert-to 'pptx:Impress MS PowerPoint 2007 XML' --outdir "$tmp" ../pptx/basic.pptx
mv "$tmp/basic.pptx" ../pptx/basic.lo.pptx
lo --convert-to odt --outdir ../docx ../docx/basic.docx
lo --convert-to odp --outdir ../pptx ../pptx/basic.pptx
rm -rf "$profile" "$tmp"
