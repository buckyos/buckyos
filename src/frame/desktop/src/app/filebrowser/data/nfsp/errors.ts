/**
 * NFSP → UiError normalization (UI_DATAMODEL.md §8.5). Backend codes map to
 * stable UI categories; raw paths, refs and server messages are filtered —
 * only the code survives into `details` as a diagnostic.
 */

import { NfspError } from '../../../../api/nfsp_client'
import type { UiError } from '../state'

interface ErrorShape {
  messageKey: string
  fallback: string
  retryable: boolean
}

const BY_CODE: Record<string, ErrorShape> = {
  NOT_FOUND: {
    messageKey: 'filebrowser.error.notFound',
    fallback: 'This item no longer exists',
    retryable: true,
  },
  STALE: {
    messageKey: 'filebrowser.error.stale',
    fallback: 'This item moved or changed — refresh to re-resolve',
    retryable: true,
  },
  PERMISSION_DENIED: {
    messageKey: 'filebrowser.error.permissionDenied',
    fallback: 'You do not have access to this item',
    retryable: false,
  },
  NAMESPACE_CONFLICT: {
    messageKey: 'filebrowser.error.nameConflict',
    fallback: 'This name already exists here',
    retryable: false,
  },
  REVISION_MISMATCH: {
    messageKey: 'filebrowser.error.conflict',
    fallback: 'The folder changed while you were working — refresh and retry',
    retryable: true,
  },
  TARGET_MISMATCH: {
    messageKey: 'filebrowser.error.conflict',
    fallback: 'The item changed while you were working — refresh and retry',
    retryable: true,
  },
  SEQ_OUT_OF_WINDOW: {
    messageKey: 'filebrowser.error.session',
    fallback: 'The session was interrupted — please retry',
    retryable: true,
  },
  NOT_EMPTY: {
    messageKey: 'filebrowser.error.notEmpty',
    fallback: 'The folder is not empty',
    retryable: false,
  },
  NOT_A_CONTAINER: {
    messageKey: 'filebrowser.error.notAContainer',
    fallback: 'This item cannot be opened as a folder',
    retryable: false,
  },
  NEED_PULL: {
    messageKey: 'filebrowser.error.needUpload',
    fallback: 'The content still needs to be uploaded',
    retryable: true,
  },
  UNSUPPORTED: {
    messageKey: 'filebrowser.error.unsupported',
    fallback: 'This operation is not supported by the server',
    retryable: false,
  },
  INVALID_ARGUMENT: {
    messageKey: 'filebrowser.error.invalidRequest',
    fallback: 'The server rejected this request',
    retryable: false,
  },
}

/** Normalize any thrown value from the NFSP client stack into a UiError. */
export function nfspToUiError(err: unknown): UiError {
  if (isUiErrorLike(err)) return err
  const code = backendCodeOf(err)
  if (code !== null) {
    const shape = BY_CODE[code] ?? {
      messageKey: 'filebrowser.error.backend',
      fallback: 'The file service reported an error',
      retryable: true,
    }
    return { code, ...shape, details: { code } }
  }
  // fetch/network-level failure: retryable connection state.
  return {
    code: 'NETWORK',
    messageKey: 'filebrowser.error.network',
    fallback: 'Cannot reach the file service',
    retryable: true,
  }
}

/** NfspError instances and `{code, message}` literals both carry a backend code. */
function backendCodeOf(err: unknown): string | null {
  if (err instanceof NfspError) return err.code
  if (
    typeof err === 'object' &&
    err !== null &&
    'code' in err &&
    typeof (err as { code: unknown }).code === 'string'
  ) {
    return (err as { code: string }).code
  }
  return null
}

function isUiErrorLike(value: unknown): value is UiError {
  return (
    typeof value === 'object' &&
    value !== null &&
    'code' in value &&
    'messageKey' in value &&
    'fallback' in value &&
    'retryable' in value
  )
}

/**
 * Reader-path variant: FileItemList carries `Error` objects, so wrap the
 * normalized fallback into an Error while keeping the code prefix out of
 * user-visible copy (components render via toUiError → fallback).
 */
export function nfspToError(err: unknown): Error {
  const ui = nfspToUiError(err)
  const wrapped = new Error(ui.fallback)
  wrapped.name = ui.code
  return wrapped
}
