"""pptx fixtures for the Office importer (Office 导入 TODO O0), made with python-pptx: a title slide and a bullet slide
(placeholders inheriting position and text style from layout and master), preset shapes with theme colours and
brightness (lumMod / lumOff), a gradient, a rotated and a mirrored shape, a straight connector, a text box with its
own size / colour / alignment, a freeform (custGeom); a cropped and a plain picture; a table with a merged cell; a
group scaled by its child extents; a bar chart; a hidden slide; a slide with its own background and speaker notes.

Run from this folder with python-pptx and Pillow: python3 make_pptx.py   (writes ../pptx/basic.pptx)"""

import io

from lxml import etree
from PIL import Image, ImageDraw
from pptx import Presentation
from pptx.chart.data import CategoryChartData
from pptx.dml.color import RGBColor
from pptx.enum.chart import XL_CHART_TYPE
from pptx.enum.dml import MSO_THEME_COLOR
from pptx.enum.shapes import MSO_CONNECTOR, MSO_SHAPE
from pptx.enum.text import MSO_ANCHOR, PP_ALIGN
from pptx.util import Emu, Inches, Pt


def picture(w, h, color):
    img = Image.new('RGB', (w, h), color)
    d = ImageDraw.Draw(img)
    for k in range(0, w, 40):
        d.line([k, 0, k, h], fill=(255, 255, 255), width=2)
    d.ellipse([w // 4, h // 4, 3 * w // 4, 3 * h // 4], fill=(250, 204, 21))
    out = io.BytesIO()
    img.save(out, 'PNG')
    out.seek(0)
    return out


prs = Presentation()
prs.slide_width = Inches(13.333)
prs.slide_height = Inches(7.5)
prs.core_properties.title = 'Office Import Deck'
layouts = prs.slide_layouts

# 1 title slide
s = prs.slides.add_slide(layouts[0])
s.shapes.title.text = 'Office Import Deck'
s.placeholders[1].text = '生成的测试幻灯片'

# 2 bullets with levels
s = prs.slides.add_slide(layouts[1])
s.shapes.title.text = 'Agenda'
tf = s.placeholders[1].text_frame
tf.text = 'Why import Office files'
for text, level in [('Editable pages', 1), ('Pictures and shapes', 2), ('High fidelity with LibreOffice', 0)]:
    p = tf.add_paragraph()
    p.text = text
    p.level = level
tf.paragraphs[0].runs[0].font.bold = True

# 3 shapes
s = prs.slides.add_slide(layouts[5])
s.shapes.title.text = 'Shapes'
r = s.shapes.add_shape(MSO_SHAPE.RECTANGLE, Inches(0.5), Inches(1.6), Inches(2.4), Inches(1.4))
r.fill.solid(); r.fill.fore_color.rgb = RGBColor(0x25, 0x63, 0xEB)
r.line.color.rgb = RGBColor(0x1E, 0x3A, 0x8A); r.line.width = Pt(2)
r.text_frame.text = 'Blue box'
r.text_frame.paragraphs[0].alignment = PP_ALIGN.CENTER
rr = s.shapes.add_shape(MSO_SHAPE.ROUNDED_RECTANGLE, Inches(3.3), Inches(1.6), Inches(2.4), Inches(1.4))
rr.fill.solid(); rr.fill.fore_color.theme_color = MSO_THEME_COLOR.ACCENT_2; rr.fill.fore_color.brightness = 0.4
rr.text_frame.text = 'Accent 2, lighter'
o = s.shapes.add_shape(MSO_SHAPE.OVAL, Inches(6.1), Inches(1.6), Inches(1.6), Inches(1.4))
o.fill.solid(); o.fill.fore_color.theme_color = MSO_THEME_COLOR.ACCENT_6; o.fill.fore_color.brightness = -0.25
s.shapes.add_shape(MSO_SHAPE.ISOSCELES_TRIANGLE, Inches(8.1), Inches(1.6), Inches(1.6), Inches(1.4))
arrow = s.shapes.add_shape(MSO_SHAPE.RIGHT_ARROW, Inches(10.1), Inches(1.6), Inches(2.4), Inches(1.2))
arrow.rotation = 30
star = s.shapes.add_shape(MSO_SHAPE.STAR_5_POINT, Inches(0.5), Inches(3.6), Inches(1.8), Inches(1.8))
star.fill.gradient(); star.fill.gradient_angle = 45
star.fill.gradient_stops[0].color.rgb = RGBColor(0xF5, 0x9E, 0x0B)
star.fill.gradient_stops[1].color.rgb = RGBColor(0xDC, 0x26, 0x26)
flipped = s.shapes.add_shape(MSO_SHAPE.PENTAGON, Inches(2.8), Inches(3.8), Inches(2.0), Inches(1.0))
flipped._element.spPr.find('{http://schemas.openxmlformats.org/drawingml/2006/main}xfrm').set('flipH', '1')
line = s.shapes.add_connector(MSO_CONNECTOR.STRAIGHT, Inches(5.2), Inches(3.8), Inches(7.4), Inches(5.2))
line.line.color.rgb = RGBColor(0x11, 0x18, 0x27); line.line.width = Pt(3)
tb = s.shapes.add_textbox(Inches(7.8), Inches(3.6), Inches(4.8), Inches(1.6))
tb.text_frame.word_wrap = True
tb.text_frame.vertical_anchor = MSO_ANCHOR.MIDDLE
p = tb.text_frame.paragraphs[0]
p.alignment = PP_ALIGN.RIGHT
run = p.add_run(); run.text = 'Right aligned, 28 pt, red'
run.font.size = Pt(28); run.font.bold = True; run.font.color.rgb = RGBColor(0xDC, 0x26, 0x26)
p2 = tb.text_frame.add_paragraph(); p2.text = 'Second paragraph'; p2.alignment = PP_ALIGN.RIGHT
ff = s.shapes.build_freeform(1.0, 6.0, scale=Inches(1))
ff.add_line_segments([(2.0, 6.0), (2.5, 6.8), (1.5, 7.2), (0.8, 6.6)], close=True)
free = ff.convert_to_shape()
free.fill.solid(); free.fill.fore_color.rgb = RGBColor(0x10, 0xB9, 0x81)

# 4 pictures (one cropped)
s = prs.slides.add_slide(layouts[5])
s.shapes.title.text = 'Pictures'
pic = s.shapes.add_picture(picture(800, 400, (124, 58, 237)), Inches(0.8), Inches(1.8), Inches(5.0))
pic.crop_left = 0.25
pic.crop_right = 0.1
s.shapes.add_picture(picture(600, 600, (5, 150, 105)), Inches(7.0), Inches(1.8), Inches(4.0))

# 5 table with a merged cell
s = prs.slides.add_slide(layouts[5])
s.shapes.title.text = 'Table'
t = s.shapes.add_table(3, 3, Inches(1), Inches(1.8), Inches(9), Inches(2.4)).table
for c, h in enumerate(['Item', 'Count', 'Note']):
    t.cell(0, c).text = h
for r, row in enumerate([['Apples', '3', 'fresh'], ['Pears', '5', '']], start=1):
    for c, v in enumerate(row):
        t.cell(r, c).text = v
t.cell(2, 2).merge(t.cell(1, 2))

# 6 a group scaled through chOff / chExt
s = prs.slides.add_slide(layouts[6])
g = s.shapes.add_group_shape()
a = g.shapes.add_shape(MSO_SHAPE.RECTANGLE, Inches(1), Inches(1), Inches(2), Inches(2))
a.text_frame.text = 'In group'
b = g.shapes.add_shape(MSO_SHAPE.OVAL, Inches(3.5), Inches(1), Inches(2), Inches(2))
ns = {'a': 'http://schemas.openxmlformats.org/drawingml/2006/main', 'p': 'http://schemas.openxmlformats.org/presentationml/2006/main'}
xfrm = g._element.find('p:grpSpPr/a:xfrm', ns)
xfrm.find('a:off', ns).set('x', str(Inches(2))); xfrm.find('a:off', ns).set('y', str(Inches(3)))
xfrm.find('a:ext', ns).set('cx', str(Inches(9))); xfrm.find('a:ext', ns).set('cy', str(Inches(4)))

# 7 a bar chart
s = prs.slides.add_slide(layouts[5])
s.shapes.title.text = 'Chart'
data = CategoryChartData(); data.categories = ['Q1', 'Q2', 'Q3']; data.add_series('Sales', (4.3, 2.5, 3.5))
s.shapes.add_chart(XL_CHART_TYPE.COLUMN_CLUSTERED, Inches(2), Inches(1.6), Inches(8), Inches(5), data)

# 8 hidden
s = prs.slides.add_slide(layouts[5])
s.shapes.title.text = 'Hidden slide'
s._element.set('show', '0')

# 9 own background + notes
s = prs.slides.add_slide(layouts[5])
s.shapes.title.text = 'Dark background'
s.background.fill.solid(); s.background.fill.fore_color.rgb = RGBColor(0x0F, 0x17, 0x2A)
s.shapes.title.text_frame.paragraphs[0].runs[0].font.color.rgb = RGBColor(0xFF, 0xFF, 0xFF)
s.notes_slide.notes_text_frame.text = 'Speaker notes are dropped.'

prs.save('../pptx/basic.pptx')
