/**
 * Internal Dialog primitive — wraps @radix-ui/react-dialog with Pensieve pv-
 * class prefix so it is isolated from the host app's CSS. Portal targets
 * usePortalContainer() so it stays inside .pensieve-root CSS variable scope.
 * NOT exported from the public package surface.
 */

import * as React from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import { cn } from "../cn";
import { usePortalContainer } from "../use-portal-container";

const Dialog = DialogPrimitive.Root;
const DialogTrigger = DialogPrimitive.Trigger;
const DialogClose = DialogPrimitive.Close;

function DialogPortal({ children }: { children: React.ReactNode }) {
  const container = usePortalContainer();
  return (
    <DialogPrimitive.Portal container={container ?? undefined}>
      {children}
    </DialogPrimitive.Portal>
  );
}

const DialogOverlay = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Overlay>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Overlay>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Overlay
    ref={ref}
    className={cn(
      "pv-fixed pv-inset-0 pv-z-50 pv-bg-black/60 data-[state=open]:pv-animate-in data-[state=closed]:pv-animate-out data-[state=closed]:pv-fade-out-0 data-[state=open]:pv-fade-in-0",
      className,
    )}
    {...props}
  />
));
DialogOverlay.displayName = "DialogOverlay";

const DialogContent = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Content>
>(({ className, children, ...props }, ref) => (
  <DialogPortal>
    <DialogOverlay />
    <DialogPrimitive.Content
      ref={ref}
      className={cn(
        "pv-fixed pv-left-1/2 pv-top-1/2 pv-z-50 pv-grid pv-w-full pv-max-w-lg -pv-translate-x-1/2 -pv-translate-y-1/2 pv-gap-4 pv-border pv-border-border pv-bg-background pv-p-6 pv-shadow-xl pv-duration-200 data-[state=open]:pv-animate-in data-[state=closed]:pv-animate-out data-[state=closed]:pv-fade-out-0 data-[state=open]:pv-fade-in-0 data-[state=closed]:pv-zoom-out-95 data-[state=open]:pv-zoom-in-95 data-[state=closed]:pv-slide-out-to-left-1/2 data-[state=closed]:pv-slide-out-to-top-[48%] data-[state=open]:pv-slide-in-from-left-1/2 data-[state=open]:pv-slide-in-from-top-[48%] pv-rounded-lg",
        className,
      )}
      {...props}
    >
      {children}
      <DialogPrimitive.Close className="pv-absolute pv-right-4 pv-top-4 pv-rounded-sm pv-opacity-70 pv-ring-offset-background pv-transition-opacity hover:pv-opacity-100 focus:pv-outline-none focus:pv-ring-2 focus:pv-ring-ring focus:pv-ring-offset-2 disabled:pv-pointer-events-none">
        <X className="pv-h-4 pv-w-4" />
        <span className="pv-sr-only">Close</span>
      </DialogPrimitive.Close>
    </DialogPrimitive.Content>
  </DialogPortal>
));
DialogContent.displayName = "DialogContent";

function DialogHeader({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn("pv-flex pv-flex-col pv-space-y-1.5 pv-text-center sm:pv-text-left", className)}
      {...props}
    />
  );
}
DialogHeader.displayName = "DialogHeader";

function DialogFooter({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "pv-flex pv-flex-col-reverse sm:pv-flex-row sm:pv-justify-end sm:pv-space-x-2",
        className,
      )}
      {...props}
    />
  );
}
DialogFooter.displayName = "DialogFooter";

const DialogTitle = React.forwardRef<
  React.ComponentRef<typeof DialogPrimitive.Title>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Title>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Title
    ref={ref}
    className={cn("pv-text-lg pv-font-semibold pv-leading-none pv-tracking-tight", className)}
    {...props}
  />
));
DialogTitle.displayName = "DialogTitle";

export {
  Dialog,
  DialogTrigger,
  DialogClose,
  DialogPortal,
  DialogOverlay,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
};
