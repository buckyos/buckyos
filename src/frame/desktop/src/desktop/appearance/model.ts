/* The account's appearance (`shared/appearance/model.ts`) with the languages of this UI. */

import { supportedLocales, type SupportedLocale } from '../../models/ui'
import { readAppearance as readAccountAppearance, type AccountAppearance as SharedAppearance } from '../../shared/appearance/model.ts'

export {
  APPEARANCE_STATE_KEY,
  appearanceFields,
  planSync,
  sameValue,
  type AppearanceField,
} from '../../shared/appearance/model.ts'

export type AccountAppearance = SharedAppearance<SupportedLocale>

export const readAppearance = (raw: unknown): Partial<AccountAppearance> => readAccountAppearance(raw, supportedLocales)
