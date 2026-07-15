export function withMockLatency<T>(value: T, delay = 140): Promise<T> {
  return new Promise((resolve) => {
    window.setTimeout(() => resolve(value), delay)
  })
}
