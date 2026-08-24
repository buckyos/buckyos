import math
import struct
import wave
import zlib
from pathlib import Path


ROOT = Path(__file__).resolve().parent


class Canvas:
    def __init__(self, width: int, height: int, color: tuple[int, int, int]):
        self.width = width
        self.height = height
        self.pixels = bytearray(color * (width * height))

    def pixel(self, x: int, y: int, color: tuple[int, int, int]) -> None:
        if 0 <= x < self.width and 0 <= y < self.height:
            offset = (y * self.width + x) * 3
            self.pixels[offset:offset + 3] = bytes(color)

    def rect(self, x0: int, y0: int, x1: int, y1: int, color: tuple[int, int, int]) -> None:
        for y in range(max(0, y0), min(self.height, y1)):
            for x in range(max(0, x0), min(self.width, x1)):
                self.pixel(x, y, color)

    def ellipse(self, cx: int, cy: int, rx: int, ry: int, color: tuple[int, int, int]) -> None:
        for y in range(max(0, cy - ry), min(self.height, cy + ry + 1)):
            span = rx * math.sqrt(max(0.0, 1.0 - ((y - cy) / ry) ** 2))
            for x in range(max(0, math.ceil(cx - span)), min(self.width, math.floor(cx + span) + 1)):
                self.pixel(x, y, color)

    def line(self, x0: int, y0: int, x1: int, y1: int, width: int, color: tuple[int, int, int]) -> None:
        steps = max(abs(x1 - x0), abs(y1 - y0), 1)
        for step in range(steps + 1):
            x = round(x0 + (x1 - x0) * step / steps)
            y = round(y0 + (y1 - y0) * step / steps)
            self.ellipse(x, y, width, width, color)

    def polygon(self, points: list[tuple[int, int]], color: tuple[int, int, int]) -> None:
        min_y = max(0, min(y for _, y in points))
        max_y = min(self.height - 1, max(y for _, y in points))
        for y in range(min_y, max_y + 1):
            intersections = []
            for index, (x0, y0) in enumerate(points):
                x1, y1 = points[(index + 1) % len(points)]
                if (y0 <= y < y1) or (y1 <= y < y0):
                    intersections.append(x0 + (y - y0) * (x1 - x0) / (y1 - y0))
            intersections.sort()
            for index in range(0, len(intersections) - 1, 2):
                for x in range(math.ceil(intersections[index]), math.floor(intersections[index + 1]) + 1):
                    self.pixel(x, y, color)


def png_bytes(canvas: Canvas) -> bytes:
    rows = b"".join(
        b"\x00" + bytes(canvas.pixels[y * canvas.width * 3:(y + 1) * canvas.width * 3])
        for y in range(canvas.height)
    )

    def chunk(kind: bytes, payload: bytes) -> bytes:
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload))

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", canvas.width, canvas.height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(rows, 9))
        + chunk(b"IEND", b"")
    )


def save_png(name: str, canvas: Canvas) -> None:
    (ROOT / name).write_bytes(png_bytes(canvas))


def primary_image() -> Canvas:
    canvas = Canvas(640, 480, (210, 226, 205))
    canvas.rect(0, 300, 640, 480, (151, 126, 92))
    for cx, cy, rx, ry, color in [
        (90, 120, 130, 100, (151, 125, 103)),
        (285, 90, 150, 105, (174, 145, 116)),
        (520, 135, 165, 130, (139, 115, 96)),
        (115, 360, 125, 85, (194, 170, 129)),
        (515, 390, 150, 100, (184, 157, 113)),
    ]:
        canvas.ellipse(cx, cy, rx, ry, color)
    canvas.line(330, 420, 330, 250, 6, (46, 112, 58))
    canvas.line(330, 345, 270, 315, 4, (48, 126, 62))
    canvas.line(330, 365, 395, 330, 4, (48, 126, 62))
    canvas.ellipse(275, 315, 38, 13, (61, 143, 71))
    canvas.ellipse(397, 330, 38, 13, (55, 135, 68))
    for angle in range(0, 360, 45):
        radians = math.radians(angle)
        canvas.ellipse(
            round(330 + math.cos(radians) * 64),
            round(225 + math.sin(radians) * 45),
            42,
            24,
            (236, 127, 184),
        )
    canvas.ellipse(330, 225, 31, 31, (247, 202, 46))
    canvas.ellipse(320, 215, 5, 5, (115, 78, 25))
    canvas.ellipse(340, 232, 5, 5, (115, 78, 25))
    return canvas


def secondary_image() -> Canvas:
    canvas = Canvas(640, 480, (128, 199, 229))
    canvas.polygon([(0, 300), (145, 125), (285, 300)], (65, 112, 121))
    canvas.polygon([(155, 300), (350, 80), (545, 300)], (75, 126, 113))
    canvas.polygon([(390, 300), (525, 145), (640, 280), (640, 340)], (54, 105, 100))
    canvas.polygon([(0, 270), (640, 255), (640, 480), (0, 480)], (109, 151, 83))
    road = [(0, 430), (180, 360), (465, 390), (640, 315)]
    for start, end in zip(road, road[1:]):
        canvas.line(*start, *end, 27, (69, 73, 76))
        canvas.line(*start, *end, 3, (245, 211, 73))
    canvas.ellipse(175, 357, 12, 7, (209, 54, 45))
    canvas.rect(165, 348, 185, 357, (221, 62, 48))
    return canvas


FONT = {
    "B": ["11110", "10001", "11110", "10001", "10001", "10001", "11110"],
    "U": ["10001", "10001", "10001", "10001", "10001", "10001", "01110"],
    "C": ["01111", "10000", "10000", "10000", "10000", "10000", "01111"],
    "K": ["10001", "10010", "10100", "11000", "10100", "10010", "10001"],
    "Y": ["10001", "01010", "00100", "00100", "00100", "00100", "00100"],
    "O": ["01110", "10001", "10001", "10001", "10001", "10001", "01110"],
    "S": ["01111", "10000", "10000", "01110", "00001", "00001", "11110"],
    "D": ["11110", "10001", "10001", "10001", "10001", "10001", "11110"],
    "V": ["10001", "10001", "10001", "10001", "10001", "01010", "00100"],
    "-": ["00000", "00000", "00000", "11111", "00000", "00000", "00000"],
    "2": ["01110", "10001", "00001", "00010", "00100", "01000", "11111"],
    "4": ["00010", "00110", "01010", "10010", "11111", "00010", "00010"],
    "7": ["11111", "00001", "00010", "00100", "01000", "01000", "01000"],
    "8": ["01110", "10001", "10001", "01110", "10001", "10001", "01110"],
}


def ocr_image() -> Canvas:
    text = "BUCKYOS-DV-4827"
    scale = 12
    spacing = scale
    width = len(text) * (5 * scale + spacing) + 2 * scale
    canvas = Canvas(width, 132, (250, 250, 246))
    x = scale
    for char in text:
        for row, bits in enumerate(FONT[char]):
            for col, bit in enumerate(bits):
                if bit == "1":
                    canvas.rect(x + col * scale, 24 + row * scale, x + (col + 1) * scale, 24 + (row + 1) * scale, (17, 29, 42))
        x += 5 * scale + spacing
    return canvas


def sfx_audio() -> None:
    sample_rate = 16_000
    duration = 4.0
    frames = bytearray()
    for index in range(round(sample_rate * duration)):
        t = index / sample_rate
        hum = 0.10 * math.sin(2 * math.pi * 110 * t)
        chirp_window = 1.0 if (0.35 < t < 0.65) or (1.55 < t < 1.85) or (2.8 < t < 3.2) else 0.0
        chirp = chirp_window * 0.42 * math.sin(2 * math.pi * (700 + 260 * t) * t)
        knock = 0.0
        for onset in (1.0, 2.35, 3.55):
            delta = t - onset
            if 0 <= delta < 0.09:
                knock += 0.65 * math.exp(-38 * delta) * math.sin(2 * math.pi * 165 * delta)
        value = max(-1.0, min(1.0, hum + chirp + knock))
        frames.extend(struct.pack("<h", round(value * 32767)))
    with wave.open(str(ROOT / "audio_sfx.wav"), "wb") as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(sample_rate)
        output.writeframes(frames)


def main() -> None:
    save_png("image_primary.png", primary_image())
    save_png("image_secondary.png", secondary_image())
    save_png("image_ocr.png", ocr_image())
    sfx_audio()


if __name__ == "__main__":
    main()
