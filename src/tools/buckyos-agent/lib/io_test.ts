import { resolveInputResource } from "./io.ts";

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
