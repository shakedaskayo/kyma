/**
 * Internal Dialog primitive — wraps @radix-ui/react-dialog with Kyma ky-
 * class prefix so it is isolated from the host app's CSS. Portal targets
 * usePortalContainer() so it stays inside .kyma-root CSS variable scope.
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
      "ky-fixed ky-inset-0 ky-z-50 ky-bg-black/60 data-[state=open]:ky-animate-in data-[state=closed]:ky-animate-out data-[state=closed]:ky-fade-out-0 data-[state=open]:ky-fade-in-0",
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
        "ky-fixed ky-left-1/2 ky-top-1/2 ky-z-50 ky-grid ky-w-full ky-max-w-lg -ky-translate-x-1/2 -ky-translate-y-1/2 ky-gap-4 ky-border ky-border-border ky-bg-background ky-p-6 ky-shadow-xl ky-duration-200 data-[state=open]:ky-animate-in data-[state=closed]:ky-animate-out data-[state=closed]:ky-fade-out-0 data-[state=open]:ky-fade-in-0 data-[state=closed]:ky-zoom-out-95 data-[state=open]:ky-zoom-in-95 data-[state=closed]:ky-slide-out-to-left-1/2 data-[state=closed]:ky-slide-out-to-top-[48%] data-[state=open]:ky-slide-in-from-left-1/2 data-[state=open]:ky-slide-in-from-top-[48%] ky-rounded-lg",
        className,
      )}
      {...props}
    >
      {children}
      <DialogPrimitive.Close className="ky-absolute ky-right-4 ky-top-4 ky-rounded-sm ky-opacity-70 ky-ring-offset-background ky-transition-opacity hover:ky-opacity-100 focus:ky-outline-none focus:ky-ring-2 focus:ky-ring-ring focus:ky-ring-offset-2 disabled:ky-pointer-events-none">
        <X className="ky-h-4 ky-w-4" />
        <span className="ky-sr-only">Close</span>
      </DialogPrimitive.Close>
    </DialogPrimitive.Content>
  </DialogPortal>
));
DialogContent.displayName = "DialogContent";

function DialogHeader({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn("ky-flex ky-flex-col ky-space-y-1.5 ky-text-center sm:ky-text-left", className)}
      {...props}
    />
  );
}
DialogHeader.displayName = "DialogHeader";

function DialogFooter({ className, ...props }: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "ky-flex ky-flex-col-reverse sm:ky-flex-row sm:ky-justify-end sm:ky-space-x-2",
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
    className={cn("ky-text-lg ky-font-semibold ky-leading-none ky-tracking-tight", className)}
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
