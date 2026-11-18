import SteamAccountsPanel from "../components/SteamAccountsPanel.jsx";

export default function SteamAccountsPage() {
  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-4xl px-5 py-6 sm:px-8">
        <SteamAccountsPanel />
      </div>
    </div>
  );
}
