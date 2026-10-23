// Selected local files stay byte-for-byte readable in the interactive mock.
const objects = new Map<string, { body: Record<string, string> } | File>()
const urls = new Map<string, string>()

export function registerMockAttachment(selected: File | readonly File[], directory: boolean): string {
  const files = Array.isArray(selected) ? selected : [selected as File]
  const putFile = (file: File) => {
    const id = `cyfile:mock-upload-${crypto.randomUUID()}`
    objects.set(id, file)
    return id
  }
  if (!directory) return putFile(files[0])
  const root = `cydir:mock-upload-${crypto.randomUUID()}`
  objects.set(root, { body: {} })
  for (const file of files) {
    const parts = file.webkitRelativePath.split('/').slice(1)
    let id = root
    for (const name of parts.slice(0, -1)) {
      const parent = objects.get(id) as { body: Record<string, string> }
      if (!parent.body[name]) {
        parent.body[name] = `cydir:mock-upload-${crypto.randomUUID()}`
        objects.set(parent.body[name], { body: {} })
      }
      id = parent.body[name]
    }
    ;(objects.get(id) as { body: Record<string, string> }).body[file.name] = putFile(file)
  }
  return root
}

export function mockAttachmentUrl(id: string): string | undefined {
  const file = objects.get(id)
  if (!(file instanceof File)) return undefined
  let url = urls.get(id)
  if (!url) { url = URL.createObjectURL(file); urls.set(id, url) }
  return url
}

export async function readMockAttachment(id: string): Promise<Response> {
  const object = objects.get(id)
  return object ? new Response(object instanceof File ? object : JSON.stringify(object)) : new Response('Object unavailable', { status: 404 })
}
