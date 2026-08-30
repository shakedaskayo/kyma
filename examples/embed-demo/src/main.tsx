import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./host.css";
import "@pensieve-ai/react/styles.css";
import { App } from "./App";

const root = document.getElementById("root");
if (!root) throw new Error("Missing #root element");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
