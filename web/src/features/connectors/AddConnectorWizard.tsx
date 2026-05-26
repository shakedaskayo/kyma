import { useMemo, useState } from "react";
import { ArrowLeft } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { useSession } from "@/sdk/session";
import {
  CONNECTOR_KINDS,
  githubKind,
  type ConnectorKindDef,
  type KindFormValues,
} from "./connector-kinds";
import { KindPicker } from "./KindPicker";
import { ConnectorConfigForm } from "./ConnectorConfigForm";
import { RepoPicker } from "./RepoPicker";
import { useCreateConnector } from "./useConnectors";

type Step = "kind" | "config" | "resources";

function blankValues(kind: ConnectorKindDef): KindFormValues {
  return {
    name: "",
    scheduleMs: kind.defaultScheduleMs,
    fields: {},
    targetDatabase: "",
    targetTable: "",
    resources: [],
  };
}

export function AddConnectorWizard({
  open,
  onClose,
  onCreated,
}: {
  open: boolean;
  onClose: () => void;
  onCreated: (id: string) => void;
}) {
  const { database } = useSession();
  const create = useCreateConnector();

  // v1 auto-skips the kind step (only GitHub registered).
  const onlyKind = CONNECTOR_KINDS.length === 1 ? CONNECTOR_KINDS[0] : null;
  const [kind, setKind] = useState<ConnectorKindDef>(onlyKind ?? githubKind);
  const [step, setStep] = useState<Step>(onlyKind ? "config" : "kind");
  const [values, setValues] = useState<KindFormValues>(() =>
    blankValues(onlyKind ?? githubKind),
  );
  const [advancedOpen, setAdvancedOpen] = useState(false);

  const reset = () => {
    const k = onlyKind ?? githubKind;
    setKind(k);
    setStep(onlyKind ? "config" : "kind");
    setValues(blankValues(k));
    setAdvancedOpen(false);
  };

  const close = () => {
    reset();
    onClose();
  };

  const tokenFieldValue = kind.resourceTokenField
    ? values.fields[kind.resourceTokenField] ?? ""
    : "";

  const update = (patch: Partial<KindFormValues>) =>
    setValues((v) => ({ ...v, ...patch }));

  const configValid = useMemo(() => {
    if (!values.name.trim()) return false;
    return kind.fields.every((f) => !f.required || (values.fields[f.key] ?? "").trim());
  }, [kind, values]);

  const handleNextFromConfig = () => {
    if (!configValid) return;
    if (kind.resourceStep) setStep("resources");
    else handleCreate();
  };

  const handleCreate = () => {
    const body = kind.buildCreateBody(values, database);
    create.mutate(body, {
      onSuccess: (res) => {
        close();
        onCreated(res.id);
      },
    });
  };

  const resourcesValid = !kind.resourceStep || values.resources.length > 0;

  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent className="max-w-xl max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {step !== "kind" && !onlyKind && (
              <button
                type="button"
                className="rounded p-0.5 text-muted-foreground hover:bg-accent"
                onClick={() =>
                  setStep(step === "resources" ? "config" : "kind")
                }
              >
                <ArrowLeft className="h-4 w-4" />
              </button>
            )}
            {step === "kind" && "Add connector"}
            {step === "config" && `Add ${kind.label} connector`}
            {step === "resources" && "Select repositories"}
          </DialogTitle>
        </DialogHeader>

        <div className="py-1">
          {step === "kind" && (
            <KindPicker
              selected={kind.id}
              onSelect={(k) => {
                setKind(k);
                setValues(blankValues(k));
                setStep("config");
              }}
            />
          )}

          {step === "config" && (
            <ConnectorConfigForm
              kind={kind}
              values={values}
              onChange={update}
              advancedOpen={advancedOpen}
              onToggleAdvanced={() => setAdvancedOpen((v) => !v)}
            />
          )}

          {step === "resources" && (
            <RepoPicker
              pat={tokenFieldValue}
              selected={values.resources}
              onChange={(resources) => update({ resources })}
            />
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={close}>
            Cancel
          </Button>
          {step === "config" && (
            <Button onClick={handleNextFromConfig} disabled={!configValid}>
              {kind.resourceStep ? "Next" : create.isPending ? "Creating…" : "Create"}
            </Button>
          )}
          {step === "resources" && (
            <Button
              onClick={handleCreate}
              disabled={!resourcesValid || create.isPending}
            >
              {create.isPending
                ? "Creating…"
                : `Create${values.resources.length ? ` (${values.resources.length})` : ""}`}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
