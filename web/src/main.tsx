import React from "react";
import ReactDOM from "react-dom/client";
import "./styles/globals.css";
import "react-grid-layout/css/styles.css";
import "@pensieve-ai/react/styles.css";
import { Providers } from "./app/providers";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Providers />
  </React.StrictMode>,
);
