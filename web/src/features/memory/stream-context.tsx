import { createContext, useContext } from "react";
import { useReducedMotion } from "@/lib/motion";
import { useMemoryStream } from "./useMemoryStream";

type StreamValue = ReturnType<typeof useMemoryStream>;

const StreamContext = createContext<StreamValue | null>(null);

/**
 * Hosts a SINGLE memory stream (one live-tail socket) for the whole Memory
 * surface, so the sticky header's events/min readout and the Overview's live
 * pulse rail share one connection instead of opening two.
 */
export function MemoryStreamProvider({ children }: { children: React.ReactNode }) {
  const reduce = useReducedMotion();
  const value = useMemoryStream({ reducedMotion: Boolean(reduce) });
  return <StreamContext.Provider value={value}>{children}</StreamContext.Provider>;
}

/** Read the shared stream. Falls back to a no-op shape outside the provider. */
export function useSharedMemoryStream(): StreamValue {
  return (
    useContext(StreamContext) ?? {
      events: [],
      status: "idle",
      liveStatus: null,
      eventsPerMin: 0,
      newestTs: "",
    }
  );
}
