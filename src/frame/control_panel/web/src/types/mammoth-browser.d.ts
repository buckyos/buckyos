declare module 'mammoth/mammoth.browser' {
  type ConvertToHtmlResult = {
    value: string
    messages?: Array<{
      type?: string
      message?: string
      error?: unknown
    }>
  }

  type Mammoth = {
    convertToHtml: (input: { arrayBuffer: ArrayBuffer }) => Promise<ConvertToHtmlResult>
  }

  const mammoth: Mammoth
  export default mammoth
}
