import { SUPPLEMENTAL as de } from "./de-supplemental";
import { SUPPLEMENTAL as faIR } from "./fa-IR-supplemental";
import { SUPPLEMENTAL as fr } from "./fr-supplemental";
import { SUPPLEMENTAL as ko } from "./ko-supplemental";
import { SUPPLEMENTAL as ru } from "./ru-supplemental";
import { SUPPLEMENTAL as th } from "./th-supplemental";
import { SUPPLEMENTAL as zhTW } from "./zh-TW-supplemental";
import type { SupplementalCopy } from "./supplemental-schema";

export const supplemental = Object.fromEntries(
  (Object.keys(de) as (keyof SupplementalCopy)[]).map(key => [key, { de: de[key], "fa-IR": faIR[key], fr: fr[key], ko: ko[key], th: th[key], ru: ru[key], "zh-TW": zhTW[key] }]),
) as { [K in keyof SupplementalCopy]: Record<"de" | "fa-IR" | "fr" | "ko" | "th" | "ru" | "zh-TW", SupplementalCopy[K]> };
