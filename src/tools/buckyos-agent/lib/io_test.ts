import { resolveInputResource, saveResourceToPath } from "./io.ts";

function assertEquals(actual: unknown, expected: unknown): void {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) {
    throw new Error(`expected ${expectedJson}, got ${actualJson}`);
  }
}

Deno.test("resolveInputResource infers a concrete MIME for local files", async () => {
  const path = await Deno.makeTempFile({ suffix: ".png" });
  try {
    await Deno.writeFile(path, new Uint8Array([1, 2, 3]));
    const resource = await resolveInputResource(path, "image/*");
    assertEquals(resource, {
      kind: "base64",
      mime: "image/png",
      data_base64: "AQID",
    });
  } finally {
    await Deno.remove(path);
  }
});

Deno.test("resolveInputResource preserves an explicit concrete MIME", async () => {
  const path = await Deno.makeTempFile({ suffix: ".bin" });
  try {
    await Deno.writeFile(path, new Uint8Array([1, 2, 3]));
    const resource = await resolveInputResource(path, "image/webp");
    assertEquals(resource, {
      kind: "base64",
      mime: "image/webp",
      data_base64: "AQID",
    });
  } finally {
    await Deno.remove(path);
  }
});

Deno.test("resolveInputResource preserves a direct cyfile typed id", async () => {
  assertEquals(await resolveInputResource("cyfile:abc123"), {
    kind: "named_object",
    obj_id: "cyfile:abc123",
  });
});

Deno.test("saveResourceToPath can refuse to overwrite", async () => {
  const dir = await Deno.makeTempDir();
  const path = `${dir}/resource.txt`;
  try {
    const resource = {
      kind: "base64" as const,
      mime: "text/plain",
      data_base64: "aGVsbG8=",
    };
    await saveResourceToPath(resource, path, {}, { overwrite: false });
    assertEquals(new TextDecoder().decode(await Deno.readFile(path)), "hello");
    let rejected = false;
    try {
      await saveResourceToPath(resource, path, {}, { overwrite: false });
    } catch {
      rejected = true;
    }
    assertEquals(rejected, true);
  } finally {
    await Deno.remove(dir, { recursive: true });
  }
});
