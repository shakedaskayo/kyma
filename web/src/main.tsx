import React from "react";
import ReactDOM from "react-dom/client";
import "./styles/globals.css";
import "react-grid-layout/css/styles.css";
import { Providers } from "./app/providers";
import { installAuthFetch } from "./sdk/auth-fetch";

// Transparent session refresh: a 401 from the API triggers a refresh-token
// exchange + one retry, or a redirect to /login if the session is dead.
installAuthFetch();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Providers />
  </React.StrictMode>,
);
