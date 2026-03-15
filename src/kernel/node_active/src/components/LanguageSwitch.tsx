import { Box, FormControl, MenuItem, Select, type SelectChangeEvent, Tooltip, Typography } from "@mui/material";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  changeLanguage,
  getCurrentLanguage,
  getLanguageDisplayName,
  getLanguageFlag,
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

  const renderLanguage = (lang: string) => (
    <Box sx={{ display: "flex", alignItems: "center", gap: 1.25, minWidth: 0 }}>
      <Box component="span" aria-hidden="true" sx={{ fontSize: 18, lineHeight: 1 }}>
        {getLanguageFlag(lang)}
      </Box>
      <Typography variant="body2" sx={{ fontWeight: 500, whiteSpace: "nowrap" }}>
        {getLanguageDisplayName(lang)}
      </Typography>
    </Box>
  )

  const handleChange = (event: SelectChangeEvent<string>) => {
    const lang = event.target.value
    if (getSupportedLanguages().includes(lang as SupportedLanguage)) {
      void changeLanguage(lang as SupportedLanguage)
      setValue(lang as SupportedLanguage)
    }
  }

  return (
    <Tooltip title={t("language_label", { defaultValue: "Language" })}>
      <FormControl
        size="small"
        sx={{
          minWidth: { xs: 156, sm: 176 },
          bgcolor: "background.paper",
          borderRadius: 3,
        }}
      >
        <Select
          value={value}
          onChange={handleChange}
          displayEmpty
          inputProps={{ "aria-label": t("language_label", { defaultValue: "Language" }) }}
          renderValue={(selected) => renderLanguage(selected)}
          sx={{
            borderRadius: 3,
            minHeight: 42,
            "& .MuiSelect-select": {
              display: "flex",
              alignItems: "center",
              py: 1,
            },
          }}
          MenuProps={{
            PaperProps: {
              sx: {
                mt: 1,
                borderRadius: 3,
              },
            },
          }}
        >
          {languageOptions.map((option) => (
            <MenuItem key={option.code} value={option.code}>
              {renderLanguage(option.code)}
            </MenuItem>
          ))}
        </Select>
      </FormControl>
    </Tooltip>
  )
}

export default LanguageSwitch;
