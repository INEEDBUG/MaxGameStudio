const icons = {
  steam: "steam",
  cs2: "counterstrike",
  valorant: "valorant",
  league: "leagueoflegends",
};

export default function GameIcon({ game, className = "" }) {
  const asset = `${import.meta.env.BASE_URL}brand-icons/${icons[game]}.svg`;
  return (
    <span
      aria-hidden="true"
      className={`game-icon game-icon-${game} ${className}`}
      style={{ "--game-icon-image": `url("${asset}")` }}
    />
  );
}

export function SteamIcon(props) {
  return <GameIcon game="steam" {...props} />;
}

export function CounterStrikeIcon(props) {
  return <GameIcon game="cs2" {...props} />;
}

export function ValorantIcon(props) {
  return <GameIcon game="valorant" {...props} />;
}
