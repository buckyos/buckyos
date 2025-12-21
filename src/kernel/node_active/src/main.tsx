import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "../i18n";
import "./styles.css";
import { SN_API_URL,ActiveConfig, init_active_lib } from "../active_lib";

const rootElement = document.getElementById("root");


async function bootstrap() {
  try {
    const resp = await fetch("/active_config.json", { cache: "no-cache" });
    if (resp.ok) {
      const config = (await resp.json()) as ActiveConfig;
      init_active_lib(config)
    } else {
      console.warn("active_config.json not found, using defaults");
    }
  } catch (err) {
    console.warn("Failed to load active_config.json, using defaults", err);
  }
  console.log("SN_API_URL:", SN_API_URL);
  if (rootElement) {
    ReactDOM.createRoot(rootElement).render(
      <React.StrictMode>
        <App />
      </React.StrictMode>
    );
  } else {
    console.error("Root element #root was not found");
  }
}

bootstrap();
