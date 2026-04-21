import React from "react";
import ReactDOM from "react-dom/client";
import "./styles/globals.css";
import { Providers } from "./app/providers";

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Providers />
  </React.StrictMode>,
);
