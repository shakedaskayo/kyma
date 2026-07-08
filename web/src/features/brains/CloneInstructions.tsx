import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useSession } from "@/sdk/session";
import { cloneUrl } from "@/sdk/brains";

/** Copy-paste clone block. The default copy uses a `<KYMA_TOKEN>` placeholder;
 * "copy with my token" substitutes the session token on an explicit click. */
export function CloneInstructions({ name }: { name: string }) {
  const { endpoint, token } = useSession();
  const url = cloneUrl(endpoint, name);
  const [copied, setCopied] = useState<"plain" | "token" | null>(null);

  const copy = (withToken: boolean) => {
    const text = withToken
      ? `git clone ${url.replace("://", `://kyma:${token}@`)}`
      : `git clone ${url}`;
    void navigator.clipboard.writeText(text);
    setCopied(withToken ? "token" : "plain");
    setTimeout(() => setCopied(null), 1500);
  };

  return (
    <div className="rounded-lg border bg-muted/30 p-3">
      <div className="flex items-center justify-between gap-2">
        <p className="text-xs font-medium text-muted-foreground">
          Clone this brain — password is a kyma API token
        </p>
        <div className="flex gap-1.5">
          <Button size="sm" variant="ghost" className="h-7 gap-1 px-2 text-xs" onClick={() => copy(false)}>
            {copied === "plain" ? <Check className="h-3 w-3" /> : <Copy className="h-3 w-3" />}
            copy
          </Button>
          <Button size="sm" variant="ghost" className="h-7 gap-1 px-2 text-xs" onClick={() => copy(true)}>
            {copied === "token" ? <Check className="h-3 w-3" /> : <Copy className="h-3 w-3" />}
            copy with my token
          </Button>
        </div>
      </div>
      <pre className="mt-2 overflow-x-auto rounded bg-background/80 px-3 py-2 font-mono text-xs">
        {`git clone ${url}\n# username: kyma · password: <KYMA_TOKEN>`}
      </pre>
      <p className="mt-2 text-2xs text-muted-foreground">
        Open the clone in Obsidian, grep it, or point an agent at it. <code>git pull</code> picks
        up new exports; <code>git push</code> flows edits back into memory. Or:{" "}
        <code>kyma brain clone {name}</code>.
      </p>
    </div>
  );
}
