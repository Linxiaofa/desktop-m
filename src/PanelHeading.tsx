import type { ReactNode } from "react";

/**
 * Shared panel heading. Keeping one implementation means every module gets the
 * same kicker, title, badge, and remove control without duplicating markup.
 */
export default function PanelHeading({
  kicker,
  title,
  titleId,
  description,
  badge,
  onRemove,
}: {
  kicker: string;
  title: string;
  titleId?: string;
  description?: string;
  badge?: ReactNode;
  onRemove?: () => void;
}) {
  return (
    <div className="panel-heading">
      <div className="panel-heading-text">
        <p className="section-kicker">{kicker}</p>
        <h2 id={titleId}>{title}</h2>
        {description && <p>{description}</p>}
      </div>
      <div className="panel-heading-actions">
        {badge}
        {onRemove && (
          <button
            className="module-remove"
            type="button"
            title="移除该模块"
            aria-label={`移除模块 ${title}`}
            onClick={onRemove}
          >
            ×
          </button>
        )}
      </div>
    </div>
  );
}
