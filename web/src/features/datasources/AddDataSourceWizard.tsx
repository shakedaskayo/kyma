import { useMemo, useState } from "react";
import { ArrowLeft, Check, Database, Table2, Timer } from "lucide-react";
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
import type { CatalogEntry } from "@/sdk/datasources";
import {
  authLabel,
  blankValues,
  buildCreateBody,
  formatInterval,
  isOAuth,
  offersStoredCredential,
  requiresStoredCredential,
  type KindFormValues,
} from "./datasource-kinds";
import { DataSourceCatalog } from "./DataSourceCatalog";
import { DataSourceConfigForm } from "./DataSourceConfigForm";
import { DataSourceConnectStep } from "./DataSourceConnectStep";
import { DataSourceCredentialPicker } from "./DataSourceCredentialPicker";
import { RepoPicker } from "./RepoPicker";
import { BrandIcon } from "./BrandIcon";
import { useCreateDataSource, useGitHubRepos } from "./useDataSources";
import { useCredentials } from "@/features/credentials/useCredentials";

type Step = "catalog" | "config" | "resources" | "review";

function stepsFor(entry: CatalogEntry | null): Step[] {
  const s: Step[] = ["catalog", "config"];
  if (entry?.resource) s.push("resources");
  s.push("review");
  return s;
}

function Stepper({
  steps,
  current,
  resourceLabel,
  configLabel,
}: {
  steps: Step[];
  current: Step;
  resourceLabel: string;
  configLabel: string;
}) {
  const labelFor = (s: Step): string =>
    s === "catalog" ? "Source" : s === "config" ? configLabel : s === "resources" ? resourceLabel : "Review";
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
              {labelFor(s)}
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

export function AddDataSourceWizard({
  open,
  onClose,
  onCreated,
}: {
  open: boolean;
  onClose: () => void;
  onCreated: (id: string) => void;
}) {
  const { database } = useSession();
  const create = useCreateDataSource();
  const repos = useGitHubRepos();
  const creds = useCredentials();

  const [entry, setEntry] = useState<CatalogEntry | null>(null);
  const [step, setStep] = useState<Step>("catalog");
  const [values, setValues] = useState<KindFormValues>({
    name: "",
    scheduleMs: 5 * 60_000,
    fields: {},
    targetDatabase: "",
    targetTable: "",
    resources: [],
    credentialId: null,
  });
  const [advancedOpen, setAdvancedOpen] = useState(false);
  // The token last submitted for verification — scopes verified/error feedback
  // to the *current* token so editing it clears stale state.
  const [attemptedPat, setAttemptedPat] = useState("");

  const steps = stepsFor(entry);
  const tokenField = entry?.resource?.token_field;
  const tokenFieldValue = tokenField ? values.fields[tokenField] ?? "" : "";
  const resourceLabel = entry?.resource?.label ?? "Resources";

  const verify = () => {
    const pat = tokenFieldValue.trim();
    if (!pat) return;
    setAttemptedPat(pat);
    repos.mutate(pat);
  };

  const reset = () => {
    setEntry(null);
    setStep("catalog");
    setValues({ name: "", scheduleMs: 5 * 60_000, fields: {}, targetDatabase: "", targetTable: "", resources: [], credentialId: null });
    setAdvancedOpen(false);
    setAttemptedPat("");
    create.reset();
    repos.reset();
  };

  const close = () => {
    reset();
    onClose();
  };

  const pickEntry = (e: CatalogEntry) => {
    setEntry(e);
    setValues(blankValues(e));
    setStep("config");
  };

  const update = (patch: Partial<KindFormValues>) => setValues((v) => ({ ...v, ...patch }));

  const configValid = useMemo(() => {
    if (!entry) return false;
    if (!values.name.trim()) return false;
    // OAuth data sources require a connected credential rather than form fields.
    if (isOAuth(entry)) return Boolean(values.credentialId);
    // Credential-only data sources (e.g. msfabric) require a stored credential.
    if (requiresStoredCredential(entry) && !values.credentialId) return false;
    return entry.fields.every((f) => !f.required || (values.fields[f.key] ?? "").trim());
  }, [entry, values]);
  const resourcesValid = !entry?.resource || values.resources.length > 0;

  const prevStep = () => {
    const i = steps.indexOf(step);
    if (i > 0) setStep(steps[i - 1]);
  };
  const nextStep = () => {
    const i = steps.indexOf(step);
    if (i >= steps.length - 1) return;
    const target = steps[i + 1];
    // Fetch the resource list when entering the resources step (unless already
    // fetched for this token).
    if (target === "resources" && tokenFieldValue.trim() && attemptedPat !== tokenFieldValue.trim()) {
      verify();
    }
    setStep(target);
  };

  const handleCreate = () => {
    if (!entry) return;
    const body = buildCreateBody(entry, values, database);
    create.mutate(body, {
      onSuccess: (res) => {
        close();
        onCreated(res.id);
      },
    });
  };

  const canAdvance =
    (step === "config" && configValid) || (step === "resources" && resourcesValid);

  return (
    <Dialog open={open} onOpenChange={(o) => !o && close()}>
      <DialogContent className="max-w-2xl max-h-[90vh] overflow-y-auto">
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
            {step === "catalog" || !entry ? (
              "Add a data source"
            ) : (
              <span className="flex items-center gap-2">
                <BrandIcon brand={entry.brand} size={18} />
                Add {entry.label}
              </span>
            )}
          </DialogTitle>
        </DialogHeader>

        {step !== "catalog" && entry && (
          <Stepper
            steps={steps}
            current={step}
            resourceLabel={resourceLabel}
            configLabel={isOAuth(entry) ? "Connect" : "Configure"}
          />
        )}

        <div className="py-1">
          {step === "catalog" && (
            <DataSourceCatalog selectedType={entry?.type_id} onPick={pickEntry} />
          )}

          {step === "config" && entry && (
            <DataSourceConfigForm
              entry={entry}
              values={values}
              onChange={update}
              advancedOpen={advancedOpen}
              onToggleAdvanced={() => setAdvancedOpen((v) => !v)}
              authSlot={
                isOAuth(entry) ? (
                  <DataSourceConnectStep
                    entry={entry}
                    credentialId={values.credentialId}
                    onConnected={(a) => update({ credentialId: a.credentialId })}
                    onClear={() => update({ credentialId: null })}
                  />
                ) : offersStoredCredential(entry) ? (
                  <DataSourceCredentialPicker
                    entry={entry}
                    credentialId={values.credentialId}
                    onChange={(credentialId) => update({ credentialId })}
                  />
                ) : undefined
              }
              verify={
                entry.resource
                  ? {
                      onVerify: verify,
                      isPending: repos.isPending,
                      isError: repos.isError && attemptedPat === tokenFieldValue.trim(),
                      verified:
                        repos.isSuccess && attemptedPat === tokenFieldValue.trim() && !!attemptedPat,
                      repoCount: repos.data?.length,
                    }
                  : undefined
              }
            />
          )}

          {step === "resources" && entry && (
            <RepoPicker
              repos={repos.data}
              isPending={repos.isPending}
              isError={repos.isError}
              onRefresh={verify}
              selected={values.resources}
              onChange={(resources) => update({ resources })}
            />
          )}

          {step === "review" && entry && (
            <div className="divide-y rounded-md border px-3">
              <ReviewRow icon={<BrandIcon brand={entry.brand} size={16} />} label="Source">
                {entry.label}
                <span className="ml-1 text-muted-foreground">· {authLabel(entry.auth_mode)}</span>
              </ReviewRow>
              {(isOAuth(entry) || (offersStoredCredential(entry) && values.credentialId)) && (
                <ReviewRow icon={<Check className="h-4 w-4 text-emerald-600" />} label={isOAuth(entry) ? "Account" : "Credential"}>
                  Connected
                  {(() => {
                    const label = creds.data?.find((c) => c.id === values.credentialId)?.label;
                    return label ? <span className="text-muted-foreground"> · {label}</span> : null;
                  })()}
                </ReviewRow>
              )}
              <ReviewRow icon={<Check className="h-4 w-4" />} label="Name">
                <span className="font-medium">{values.name.trim() || "—"}</span>
              </ReviewRow>
              <ReviewRow icon={<Timer className="h-4 w-4" />} label="Sync interval">
                {formatInterval(values.scheduleMs)}
              </ReviewRow>
              <ReviewRow icon={<Database className="h-4 w-4" />} label="Target database">
                <span className="font-mono text-xs">{values.targetDatabase.trim() || database}</span>
              </ReviewRow>
              {entry.graph_name && (
                <ReviewRow icon={<Table2 className="h-4 w-4" />} label="Graph tables">
                  <span className="font-mono text-xs">
                    {(values.targetTable.trim() || entry.default_target_table || "").replace(/_nodes$/, "")}
                    _nodes / _edges
                  </span>
                </ReviewRow>
              )}
              {entry.resource && (
                <ReviewRow
                  icon={<BrandIcon brand={entry.brand} size={16} monochrome />}
                  label={`${resourceLabel} (${values.resources.length})`}
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
              {(create.error as Error)?.message ?? "Failed to create data source."}
            </p>
          )}
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={close}>
            Cancel
          </Button>
          {step !== "review" && step !== "catalog" && (
            <Button onClick={nextStep} disabled={!canAdvance}>
              Next
            </Button>
          )}
          {step === "review" && (
            <Button onClick={handleCreate} disabled={create.isPending}>
              {create.isPending ? "Creating…" : "Create data source"}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
