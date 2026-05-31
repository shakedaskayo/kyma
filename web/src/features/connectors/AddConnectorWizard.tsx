import { useMemo, useState } from "react";
import { ArrowLeft, Check, Database, GitBranch, Table2, Timer } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { useSession } from "@/sdk/session";
import {
  CONNECTOR_KINDS,
  formatInterval,
  githubKind,
  type ConnectorKindDef,
  type KindFormValues,
} from "./connector-kinds";
import { KindPicker } from "./KindPicker";
import { ConnectorConfigForm } from "./ConnectorConfigForm";
import { RepoPicker } from "./RepoPicker";
import { useCreateConnector, useGitHubRepos } from "./useConnectors";

type Step = "kind" | "config" | "resources" | "review";

const STEP_LABEL: Record<Step, string> = {
  kind: "Source",
  config: "Configure",
  resources: "Repositories",
  review: "Review",
};

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

/** The ordered steps for a given kind (kind-picker shown only with >1 kind). */
function stepsFor(kind: ConnectorKindDef, multiKind: boolean): Step[] {
  const s: Step[] = [];
  if (multiKind) s.push("kind");
  s.push("config");
  if (kind.resourceStep) s.push("resources");
  s.push("review");
  return s;
}

function Stepper({ steps, current }: { steps: Step[]; current: Step }) {
  const idx = steps.indexOf(current);
  return (
    <ol className="flex items-center gap-1 px-1 pb-1 text-xs">
      {steps.map((s, i) => {
        const done = i < idx;
        const active = i === idx;
        return (
          <li key={s} className="flex items-center gap-1">
            <span
              className={cn(
                "flex h-5 w-5 items-center justify-center rounded-full border text-[10px] font-medium",
                active && "border-primary bg-primary text-primary-foreground",
                done && "border-primary bg-primary/10 text-primary",
                !active && !done && "border-input text-muted-foreground",
              )}
            >
              {done ? <Check className="h-3 w-3" /> : i + 1}
            </span>
            <span className={cn(active ? "font-medium text-foreground" : "text-muted-foreground")}>
              {STEP_LABEL[s]}
            </span>
            {i < steps.length - 1 && <span className="mx-1 h-px w-5 bg-border" />}
          </li>
        );
      })}
    </ol>
  );
}

function ReviewRow({
  icon,
  label,
  children,
}: {
  icon: React.ReactNode;
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex items-start gap-2.5 py-2">
      <span className="mt-0.5 text-muted-foreground">{icon}</span>
      <div className="min-w-0 flex-1">
        <div className="text-xs text-muted-foreground">{label}</div>
        <div className="text-sm">{children}</div>
      </div>
    </div>
  );
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
  const repos = useGitHubRepos();

  // v1 auto-skips the kind step (only GitHub registered).
  const multiKind = CONNECTOR_KINDS.length > 1;
  const onlyKind = multiKind ? null : CONNECTOR_KINDS[0] ?? githubKind;
  const [kind, setKind] = useState<ConnectorKindDef>(onlyKind ?? githubKind);
  const [step, setStep] = useState<Step>(multiKind ? "kind" : "config");
  const [values, setValues] = useState<KindFormValues>(() => blankValues(onlyKind ?? githubKind));
  const [advancedOpen, setAdvancedOpen] = useState(false);
  // The token last submitted for verification — lets us scope the verified/error
  // state to the *current* token so editing it clears stale feedback.
  const [attemptedPat, setAttemptedPat] = useState("");

  const steps = stepsFor(kind, multiKind);
  const tokenFieldValue = kind.resourceTokenField ? values.fields[kind.resourceTokenField] ?? "" : "";

  const verify = () => {
    const pat = tokenFieldValue.trim();
    if (!pat) return;
    setAttemptedPat(pat);
    repos.mutate(pat);
  };

  const reset = () => {
    const k = onlyKind ?? githubKind;
    setKind(k);
    setStep(multiKind ? "kind" : "config");
    setValues(blankValues(k));
    setAdvancedOpen(false);
    setAttemptedPat("");
    create.reset();
    repos.reset();
  };

  const close = () => {
    reset();
    onClose();
  };

  const update = (patch: Partial<KindFormValues>) => setValues((v) => ({ ...v, ...patch }));

  const configValid = useMemo(() => {
    if (!values.name.trim()) return false;
    return kind.fields.every((f) => !f.required || (values.fields[f.key] ?? "").trim());
  }, [kind, values]);
  const resourcesValid = !kind.resourceStep || values.resources.length > 0;

  const prevStep = () => {
    const i = steps.indexOf(step);
    if (i > 0) setStep(steps[i - 1]);
  };
  const nextStep = () => {
    const i = steps.indexOf(step);
    if (i >= steps.length - 1) return;
    const target = steps[i + 1];
    // Fetch the repo list when entering the Repositories step (unless already
    // fetched for this token).
    if (target === "resources" && tokenFieldValue.trim() && attemptedPat !== tokenFieldValue.trim()) {
      verify();
    }
    setStep(target);
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

  const canAdvance =
    step === "kind" ||
    (step === "config" && configValid) ||
    (step === "resources" && resourcesValid);

  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent className="max-w-xl max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {steps.indexOf(step) > 0 && (
              <button
                type="button"
                className="rounded p-0.5 text-muted-foreground hover:bg-accent"
                onClick={prevStep}
                aria-label="Back"
              >
                <ArrowLeft className="h-4 w-4" />
              </button>
            )}
            {step === "kind" ? "Add connector" : `Add ${kind.label} connector`}
          </DialogTitle>
        </DialogHeader>

        {step !== "kind" && <Stepper steps={steps} current={step} />}

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
              verify={
                kind.resourceTokenField
                  ? {
                      onVerify: verify,
                      isPending: repos.isPending,
                      isError: repos.isError && attemptedPat === tokenFieldValue.trim(),
                      verified: repos.isSuccess && attemptedPat === tokenFieldValue.trim() && !!attemptedPat,
                      repoCount: repos.data?.length,
                    }
                  : undefined
              }
            />
          )}

          {step === "resources" && (
            <RepoPicker
              repos={repos.data}
              isPending={repos.isPending}
              isError={repos.isError}
              onRefresh={verify}
              selected={values.resources}
              onChange={(resources) => update({ resources })}
            />
          )}

          {step === "review" && (
            <div className="divide-y rounded-md border px-3">
              <ReviewRow icon={<GitBranch className="h-4 w-4" />} label="Source">
                {kind.label}
                <span className="ml-1 text-muted-foreground">· PAT auth</span>
              </ReviewRow>
              <ReviewRow icon={<Check className="h-4 w-4" />} label="Name">
                <span className="font-medium">{values.name.trim() || "—"}</span>
              </ReviewRow>
              <ReviewRow icon={<Timer className="h-4 w-4" />} label="Sync interval">
                {formatInterval(values.scheduleMs)}
              </ReviewRow>
              <ReviewRow icon={<Database className="h-4 w-4" />} label="Target database">
                <span className="font-mono text-xs">{values.targetDatabase.trim() || database}</span>
              </ReviewRow>
              <ReviewRow icon={<Table2 className="h-4 w-4" />} label="Graph tables">
                <span className="font-mono text-xs">
                  {(values.targetTable.trim() || kind.defaultTargetTable).replace(/_nodes$/, "")}_nodes / _edges
                </span>
              </ReviewRow>
              {kind.resourceStep && (
                <ReviewRow
                  icon={<GitBranch className="h-4 w-4" />}
                  label={`Repositories (${values.resources.length})`}
                >
                  <div className="flex flex-wrap gap-1 pt-0.5">
                    {values.resources.slice(0, 8).map((r) => (
                      <span key={r} className="rounded bg-muted px-1.5 py-0.5 font-mono text-[11px]">
                        {r}
                      </span>
                    ))}
                    {values.resources.length > 8 && (
                      <span className="text-xs text-muted-foreground">
                        +{values.resources.length - 8} more
                      </span>
                    )}
                  </div>
                </ReviewRow>
              )}
            </div>
          )}

          {create.isError && (
            <p className="mt-3 rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">
              {(create.error as Error)?.message ?? "Failed to create connector."}
            </p>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={close}>
            Cancel
          </Button>
          {step !== "review" && step !== "kind" && (
            <Button onClick={nextStep} disabled={!canAdvance}>
              Next
            </Button>
          )}
          {step === "review" && (
            <Button onClick={handleCreate} disabled={create.isPending}>
              {create.isPending ? "Creating…" : "Create connector"}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
