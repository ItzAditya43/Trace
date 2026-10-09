import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { Mini } from "./Batch5";
import "./App.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    {new URLSearchParams(location.search).has("mini") ? <Mini /> : <App />}
  </React.StrictMode>,
);
