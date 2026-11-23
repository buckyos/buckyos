import { AtSign, Globe } from "lucide-react";
import { Box, Button, Stack } from "@mui/material";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { WEB3_BASE_HOST, customDomainReady } from "../../../active_lib";
import { WizardData } from "../../types";
import type { UpdateWizard } from "../ActiveWizard";
import CustomDomain from "../domain/CustomDomain";
import { ChoiceCard, StepActions, mono } from "../ui";

type Props = {
  wizardData: WizardData;
  onUpdate: UpdateWizard;
  onNext: () => void;
  onBack: () => void;
};

const DomainStep = ({ wizardData, onUpdate, onNext, onBack }: Props) => {
  const { t } = useTranslation();
  const [editing, setEditing] = useState(false);
  const username = wizardData.sn_user_name?.trim().toLowerCase() || "";
  const custom = wizardData.use_self_domain;
  const ready = custom ? !editing && customDomainReady(wizardData) : username !== "";
  const iconProps = { size: 20, strokeWidth: 1.75 };

  return (
    <Stack spacing={3}>
      <Box
        role="radiogroup"
        aria-label={t("title_step_domain")}
        sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", sm: "repeat(2, minmax(0, 1fr))" }, gap: 1.5 }}
      >
        <ChoiceCard
          selected={!custom}
          onSelect={() => onUpdate({ use_self_domain: false })}
          icon={<AtSign {...iconProps} />}
          title={t("use_buckyos_domain")}
          description={
            <Box component="span" sx={{ ...mono, fontSize: 13, overflowWrap: "anywhere" }}>
              {`https://${username || "$name"}.${WEB3_BASE_HOST}`}
            </Box>
          }
        />
        <ChoiceCard
          selected={custom}
          onSelect={() => onUpdate({ use_self_domain: true })}
          icon={<Globe {...iconProps} />}
          title={t("use_own_domain")}
          description={t("use_own_domain_desc")}
        />
      </Box>

      {custom ? (
        <CustomDomain wizardData={wizardData} onUpdate={onUpdate} onEditing={setEditing} />
      ) : null}

      <StepActions
        back={
          <Button variant="text" onClick={onBack}>
            {t("back_button")}
          </Button>
        }
      >
        <Button variant="contained" size="large" onClick={onNext} disabled={!ready} sx={{ minWidth: 120 }}>
          {t("next_button")}
        </Button>
      </StepActions>
    </Stack>
  );
};

export default DomainStep;
