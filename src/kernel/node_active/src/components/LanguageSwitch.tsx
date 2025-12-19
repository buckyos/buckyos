import { ToggleButton, ToggleButtonGroup, Tooltip } from "@mui/material";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  changeLanguage,
  getCurrentLanguage,
  getLanguageDisplayName,
  getSupportedLanguages,
} from "@legacy/i18n";

const LanguageSwitch = () => {
  const { i18n } = useTranslation();
  const [value, setValue] = useState(getCurrentLanguage());

  useEffect(() => {
    const handler = (lng: string) => setValue(lng);
    i18n.on("languageChanged", handler);
    return () => {
      i18n.off("languageChanged", handler);
    };
  }, [i18n]);

  const handleChange = (_: React.MouseEvent<HTMLElement>, lang: string | null) => {
    if (lang) {
      changeLanguage(lang as "en" | "zh");
      setValue(lang);
    }
  };

  return (
    <Tooltip title="Language">
      <ToggleButtonGroup
        exclusive
        size="small"
        color="primary"
        value={value}
        onChange={handleChange}
        aria-label="language switcher"
        sx={{ bgcolor: "background.paper" }}
      >
        {getSupportedLanguages().map((lang) => (
          <ToggleButton key={lang} value={lang} sx={{ px: 1.5 }}>
            {getLanguageDisplayName(lang)}
          </ToggleButton>
        ))}
      </ToggleButtonGroup>
    </Tooltip>
  );
};

export default LanguageSwitch;
