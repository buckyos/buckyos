export type ReadableNamedData = {
  openReader: (request: { obj_id: string; inner_path?: string | null }) => Promise<{
    body: ReadableStream<Uint8Array> | null;
    totalSize: number | null;
  }>;
};

export type ArtifactAudit = {
  obj_id: string;
  label?: string;
  size: number;
  sha256: string;
  archive_entries?: string[];
  metadata?: Record<string, number | string>;
};

function hex(bytes: Uint8Array): string {
  return [...bytes].map((value) => value.toString(16).padStart(2, "0")).join("");
}

function u16(view: DataView, offset: number): number {
  return view.getUint16(offset, true);
}

function u32(view: DataView, offset: number): number {
  return view.getUint32(offset, true);
}

function crc32(bytes: Uint8Array): number {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc >>> 1) ^ ((crc & 1) ? 0xedb88320 : 0);
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

async function validateZip(bytes: Uint8Array): Promise<string[]> {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let eocd = -1;
  for (let offset = Math.max(0, bytes.length - 65_557); offset <= bytes.length - 22; offset += 1) {
    if (u32(view, offset) === 0x06054b50) eocd = offset;
  }
  if (eocd < 0) throw new Error("ZIP end-of-central-directory record is missing");
  const count = u16(view, eocd + 10);
  let offset = u32(view, eocd + 16);
  const names: string[] = [];
  for (let index = 0; index < count; index += 1) {
    if (offset + 46 > bytes.length || u32(view, offset) !== 0x02014b50) {
      throw new Error(`ZIP central directory entry ${index} is malformed`);
    }
    const method = u16(view, offset + 10);
    const expectedCrc = u32(view, offset + 16);
    const compressedSize = u32(view, offset + 20);
    const uncompressedSize = u32(view, offset + 24);
    const nameLength = u16(view, offset + 28);
    const extraLength = u16(view, offset + 30);
    const commentLength = u16(view, offset + 32);
    const localOffset = u32(view, offset + 42);
    const name = new TextDecoder().decode(bytes.subarray(offset + 46, offset + 46 + nameLength));
    if (!name || name.startsWith("/") || name.includes("\\") || name.split("/").includes("..")) {
      throw new Error(`ZIP contains unsafe entry name ${JSON.stringify(name)}`);
    }
    names.push(name);
    if (!name.endsWith("/")) {
      if (localOffset + 30 > bytes.length || u32(view, localOffset) !== 0x04034b50) {
        throw new Error(`ZIP local header is missing for ${name}`);
      }
      const localNameLength = u16(view, localOffset + 26);
      const localExtraLength = u16(view, localOffset + 28);
      const dataOffset = localOffset + 30 + localNameLength + localExtraLength;
      const compressed = bytes.subarray(dataOffset, dataOffset + compressedSize);
      let decoded: Uint8Array;
      if (method === 0) decoded = compressed;
      else if (method === 8) {
        const stream = new Blob([new Uint8Array(compressed).buffer]).stream()
          .pipeThrough(new DecompressionStream("deflate-raw"));
        decoded = new Uint8Array(await new Response(stream).arrayBuffer());
      } else throw new Error(`ZIP entry ${name} uses unsupported compression method ${method}`);
      if (decoded.length !== uncompressedSize) throw new Error(`ZIP entry ${name} size mismatch`);
      if (crc32(decoded) !== expectedCrc) throw new Error(`ZIP entry ${name} CRC mismatch`);
    }
    offset += 46 + nameLength + extraLength + commentLength;
  }
  return names;
}

function artifactMetadata(bytes: Uint8Array, label = ""): Record<string, number | string> | undefined {
  const lower = label.toLowerCase();
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if ((lower.endsWith(".png") || lower.includes("image/png")) && bytes.length >= 24) {
    return { width: view.getUint32(16), height: view.getUint32(20), format: "png" };
  }
  if ((lower.endsWith(".wav") || lower.includes("audio/wav")) && bytes.length >= 44) {
    let offset = 12;
    let sampleRate = 0;
    let byteRate = 0;
    let dataSize = 0;
    while (offset + 8 <= bytes.length) {
      const chunk = new TextDecoder().decode(bytes.subarray(offset, offset + 4));
      const size = view.getUint32(offset + 4, true);
      if (chunk === "fmt " && size >= 16 && offset + 16 <= bytes.length) {
        sampleRate = view.getUint32(offset + 12, true);
        byteRate = view.getUint32(offset + 16, true);
      } else if (chunk === "data") dataSize = size;
      offset += 8 + size + (size % 2);
    }
    if (!sampleRate || !byteRate || !dataSize) throw new Error("WAV artifact has incomplete format/data metadata");
    return { sample_rate_hz: sampleRate, duration_seconds: dataSize / byteRate, format: "wav" };
  }
  if (lower.endsWith(".mp4") || lower.includes("video/mp4")) {
    const findBox = (start: number, end: number, kind: string): number => {
      let offset = start;
      while (offset + 8 <= end) {
        const size = view.getUint32(offset);
        const type = new TextDecoder().decode(bytes.subarray(offset + 4, offset + 8));
        if (type === kind) return offset;
        if (size < 8 || offset + size > end) break;
        offset += size;
      }
      return -1;
    };
    const moov = findBox(0, bytes.length, "moov");
    if (moov >= 0) {
      const moovSize = view.getUint32(moov);
      const mvhd = findBox(moov + 8, moov + moovSize, "mvhd");
      if (mvhd >= 0) {
        const version = bytes[mvhd + 8];
        const timescale = version === 1 ? view.getUint32(mvhd + 28) : view.getUint32(mvhd + 20);
        const duration = version === 1 ? Number(view.getBigUint64(mvhd + 32)) : view.getUint32(mvhd + 24);
        if (timescale > 0 && duration > 0) return { duration_seconds: duration / timescale, format: "mp4" };
      }
    }
    return { format: "mp4" };
  }
  return undefined;
}

function validateHeader(bytes: Uint8Array, label = ""): void {
  const lower = label.toLowerCase();
  const starts = (...values: number[]) => values.every((value, index) => bytes[index] === value);
  if ((lower.endsWith(".png") || lower.includes("image/png")) && !starts(0x89, 0x50, 0x4e, 0x47)) {
    throw new Error("PNG artifact has an invalid signature");
  }
  if ((lower.endsWith(".jpg") || lower.endsWith(".jpeg") || lower.includes("image/jpeg")) &&
    !starts(0xff, 0xd8, 0xff)) {
    throw new Error("JPEG artifact has an invalid signature");
  }
  if ((lower.endsWith(".pdf") || lower.includes("application/pdf")) && !starts(0x25, 0x50, 0x44, 0x46)) {
    throw new Error("PDF artifact has an invalid signature");
  }
  if ((lower.endsWith(".wav") || lower.includes("audio/wav")) && !starts(0x52, 0x49, 0x46, 0x46)) {
    throw new Error("WAV artifact has an invalid signature");
  }
  if ((lower.endsWith(".mp4") || lower.includes("video/mp4")) &&
    !(bytes.length >= 12 && new TextDecoder().decode(bytes.subarray(4, 8)) === "ftyp")) {
    throw new Error("MP4 artifact has an invalid ftyp box");
  }
}

export async function validateNamedArtifact(
  ndm: ReadableNamedData,
  input: { obj_id: string; label?: string },
): Promise<ArtifactAudit> {
  const opened = await ndm.openReader({ obj_id: input.obj_id });
  if (!opened.body) throw new Error(`named object ${input.obj_id} returned no readable body`);
  const bytes = new Uint8Array(await new Response(opened.body).arrayBuffer());
  if (bytes.length === 0) throw new Error(`named object ${input.obj_id} is empty`);
  if (opened.totalSize !== null && opened.totalSize !== bytes.length) {
    throw new Error(`named object ${input.obj_id} size mismatch: ${opened.totalSize} != ${bytes.length}`);
  }
  return await validateArtifactBytes(bytes, { id: input.obj_id, label: input.label });
}

export async function validateArtifactBytes(
  bytes: Uint8Array,
  input: { id: string; label?: string },
): Promise<ArtifactAudit> {
  if (bytes.length === 0) throw new Error(`artifact ${input.id} is empty`);
  validateHeader(bytes, input.label);
  const label = input.label?.toLowerCase() ?? "";
  const archiveEntries = label.endsWith(".zip") || label.includes("application/zip")
    ? await validateZip(bytes)
    : undefined;
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", new Uint8Array(bytes).buffer));
  const metadata = artifactMetadata(bytes, input.label);
  return {
    obj_id: input.id,
    label: input.label,
    size: bytes.length,
    sha256: hex(digest),
    ...(archiveEntries ? { archive_entries: archiveEntries } : {}),
    ...(metadata ? { metadata } : {}),
  };
}
