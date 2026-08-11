import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

interface UpdateInfo {
  current_version: string;
  latest_version: string;
  release_url: string;
  release_name: string;
}

const DISMISSED_KEY = "trace-dismissed-update-version";

export default function UpdateBanner() {
  const [update, setUpdate] = useState<UpdateInfo | null>(null);

  useEffect(() => {
    invoke<UpdateInfo | null>("check_for_update")
      .then((info) => {
        if (info && localStorage.getItem(DISMISSED_KEY) !== info.latest_version) {
          setUpdate(info);
        }
      })
      .catch(() => {
        // Offline or rate-limited — silently skip, this is a courtesy check.
      });
  }, []);

  if (!update) return null;

  function dismiss() {
    localStorage.setItem(DISMISSED_KEY, update!.latest_version);
    setUpdate(null);
  }

  return (
    <div className="update-banner">
      <span>
        New version available: <strong>{update.latest_version}</strong>{" "}
        (you have {update.current_version})
      </span>
      <div className="update-banner-actions">
        <button
          className="ctl-btn-sm"
          onClick={() => openUrl(update.release_url)}
        >
          View release
        </button>
        <button className="update-dismiss" onClick={dismiss}>
          ✕
        </button>
      </div>
    </div>
  );
}
