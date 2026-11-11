import { useLayoutEffect, useMemo, useRef } from 'react'
import { usePickFiles, type PickFilesOptions } from '../../../../sysdlg/select/pickFiles'
import { describeError } from '../../api/session'
import { useAiwsText } from '../../useAiwsText'

export interface FilePicker {
  /** Files chosen in the system file picker; null when the user cancels or choosing failed. */
  files(options?: PickFilesOptions): Promise<File[] | null>
  /** One file, as for `<input accept>`. */
  file(accept?: string): Promise<File | null>
}

/**
 * Every file the Workspace takes in is chosen in the system file picker: the Zone first, this device through the
 * picker's "This device". On a Workspace's own origin the picker is framed over the page (sysdlg path C). Zone files
 * are read into `File`s, so both sources go through the same import. A failure (the picker could not open, a Zone file
 * could not be read) is told through `onError`.
 */
export function useFilePicker(onError: (message: string) => void): FilePicker {
  const pick = usePickFiles('aiworkspace')
  const { t } = useAiwsText()
  const report = useRef(onError)
  useLayoutEffect(() => { report.current = onError })
  return useMemo(() => {
    const files = (options?: PickFilesOptions) => pick(options).catch((error: unknown) => {
      report.current(t('aiworkspace.common.pickFailed', { error: describeError(error) }))
      return null
    })
    return { files, file: async (accept?: string) => (await files({ accept }))?.[0] ?? null }
  }, [pick, t])
}
