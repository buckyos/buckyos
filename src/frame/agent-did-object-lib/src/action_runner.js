const input = Deno.stdin.readable.pipeThrough(new TextDecoderStream()).getReader()
let buffer = ""
async function receive() {
  while (!buffer.includes("\n")) {
    const {value, done} = await input.read()
    if (done) throw new Error("host disconnected")
    buffer += value
  }
  const split = buffer.indexOf("\n")
  const value = JSON.parse(buffer.slice(0, split))
  buffer = buffer.slice(split + 1)
  return value
}
const send = (message) => console.log(JSON.stringify(message))
const {script, request, context} = await receive()
let next = 0
let tail = Promise.resolve()
const host = {
  context,
  call(method, params) {
    const job = tail.then(async () => {
      const id = ++next
      send({id, method, params})
      const reply = await receive()
      if (reply.id !== id) throw new Error("unexpected host reply")
      if (reply.error) throw Object.assign(new Error(reply.error.message), reply.error)
      return reply.result
    })
    tail = job.catch(() => {})
    return job
  },
}
try {
  const {default: execute} = await import(script)
  const response = await execute(request, host)
  await tail
  send({response})
} catch (err) {
  if (typeof err?.code === "string" && !err.code.startsWith("ERR_")) send({response: {error: {code: err.code, message: err.message, details: err.details}}})
  else Deno.exit(1)
}
