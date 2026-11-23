import type { TFunction } from "i18next";
import { sn } from "buckyos";
import { activeErrorOf } from "./domain";

export type ErrorText = { message: string; detail?: string };

/** SN's own name for a failed bind never reaches the page as raw text; each name has its explanation. */
export function codeText(t: TFunction, code: string): string {
  return t(code.startsWith("bind_") ? [`code_${code}`, "code_bind_other"] : [`code_${code}`, "code_unknown"]);
}

export function errorText(t: TFunction, error: unknown): ErrorText {
  const active = activeErrorOf(error);
  if (active) {
    const known = active.code.startsWith("bind_") || t(`code_${active.code}`, { defaultValue: "" });
    return { message: codeText(t, active.code), ...(known ? {} : { detail: active.code }) };
  }
  if (error instanceof sn.SnClientError && error.codeName) {
    const known = t(`sn_error_${error.codeName}`, { defaultValue: "" });
    if (known) return { message: known };
  }
  return { message: t("error_request_failed"), detail: error instanceof Error ? error.message : String(error) };
}
