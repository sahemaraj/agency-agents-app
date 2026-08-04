import en, { type MessageKey, type Messages } from "./locales/en";

export const LOCALES = ["en", "zh-CN", "zh-TW", "ja", "ko", "es", "fr", "de", "pt-BR", "ru"] as const;
export type Locale = (typeof LOCALES)[number];

export const DEFAULT_LOCALE: Locale = "en";

export function isLocale(value: string): value is Locale {
  return (LOCALES as readonly string[]).includes(value);
}

export const localeLabels: Record<Locale, string> = {
  en: "English",
  "zh-CN": "简体中文",
  "zh-TW": "繁體中文",
  ja: "日本語",
  ko: "한국어",
  es: "Español",
  fr: "Français",
  de: "Deutsch",
  "pt-BR": "Português (Brasil)",
  ru: "Русский",
};

const loaders = {
  en: async () => ({}),
  "zh-CN": async () => (await import("./locales/zh-CN")).default,
  "zh-TW": async () => (await import("./locales/zh-TW")).default,
  ja: async () => (await import("./locales/ja")).default,
  ko: async () => (await import("./locales/ko")).default,
  es: async () => (await import("./locales/es")).default,
  fr: async () => (await import("./locales/fr")).default,
  de: async () => (await import("./locales/de")).default,
  "pt-BR": async () => (await import("./locales/pt-BR")).default,
  ru: async () => (await import("./locales/ru")).default,
} satisfies Record<Locale, () => Promise<Partial<Messages>>>;

export async function loadMessages(locale: Locale): Promise<Messages> {
  return { ...en, ...(await loaders[locale]()) };
}

export { en as defaultMessages };
export type { MessageKey, Messages };
