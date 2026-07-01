import { createFileRoute } from "@tanstack/react-router";
import { RunDetailEmpty } from "@/features/dreaming/RunDetailEmpty";

export const Route = createFileRoute("/_app/memory/dreaming/")({
  component: RunDetailEmpty,
});
