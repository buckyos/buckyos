import { FormControl, MenuItem, Select, type SelectChangeEvent } from "@mui/material";
import { Languages } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  changeLanguage,
  getCurrentLanguage,
  getLanguageDisplayName,
  getLanguageOptions,
  getSupportedLanguages,
  type SupportedLanguage,
} from "../../i18n";

const LanguageSwitch = () => {
  const { i18n, t } = useTranslation();
  const [value, setValue] = useState(getCurrentLanguage());
  const languageOptions = getLanguageOptions();

  useEffect(() => {
    const handler = (lng: string) => setValue(getCurrentLanguage());
    i18n.on("languageChanged", handler);
    return () => {
      i18n.off("languageChanged", handler);
    };
  }, [i18n]);

  const handleChange = (event: SelectChangeEvent<string>) => {
    const lang = event.target.value
    if (getSupportedLanguages().includes(lang as SupportedLanguage)) {
      void changeLanguage(lang as SupportedLanguage)
      setValue(lang as SupportedLanguage)
    }
  }

  const label = t("language_label", { defaultValue: "Language" })

  return (
    <FormControl size="small" sx={{ minWidth: 0 }}>
      <Select
        value={value}
        onChange={handleChange}
        inputProps={{ "aria-label": label }}
        renderValue={(selected) => getLanguageDisplayName(selected)}
        startAdornment={<Languages size={16} strokeWidth={1.75} style={{ flexShrink: 0, color: "var(--cp-muted)" }} />}
        sx={{
          height: 40,
          fontSize: 14,
          "& .MuiSelect-select": { pl: 1, py: 0, whiteSpace: "nowrap" },
        }}
        MenuProps={{ PaperProps: { sx: { mt: 0.5 } } }}
      >
        {languageOptions.map((option) => (
          <MenuItem key={option.code} value={option.code}>
            {getLanguageDisplayName(option.code)}
          </MenuItem>
        ))}
      </Select>
    </FormControl>
  )
}

export default LanguageSwitch;
