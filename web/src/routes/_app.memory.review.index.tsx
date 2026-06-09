import { createFileRoute } from "@tanstack/react-router";
import { ReviewInbox } from "@/features/review/ReviewInbox";

export const Route = createFileRoute("/_app/memory/review/")({
  component: ReviewInbox,
});
