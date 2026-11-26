import { fmtParam, isDefault, type ParamDef } from 'wb-client';

export interface ParamControlProps {
  def: ParamDef;
  /** What to show (draft if the user is editing, else the world's value). */
  value: number;
  error?: string;
  busy?: boolean;
  onEdit(value: number): void;
  onCommit(): void;
  onReset(): void;
}

/** One control, chosen from the schema's `widget` hint. It knows no parameter by name. */
export function ParamControl({ def, value, error, busy, onEdit, onCommit, onReset }: ParamControlProps) {
  const id = `param-${def.name}`;
  const label = def.name.replace(/_/g, ' ');
  const errId = `${id}-error`;
  const described = error ? errId : undefined;
  const common = { id, 'aria-invalid': error ? true : undefined, 'aria-describedby': described } as const;
  const isInt = def.widget === 'integer';
  const step = isInt ? 1 : (def.max - def.min) / 1000;
  let control;
  if (def.widget === 'select') {
    control = (
      <select {...common} value={Number.isFinite(value) ? value : def.default} onChange={(e) => { onEdit(Number(e.target.value)); onCommit(); }}>
        {(def.options ?? []).map((o, i) => <option key={o} value={i}>{o}</option>)}
      </select>
    );
  } else {
    const showRange = !isInt || def.max - def.min <= 20;
    control = (
      <div className="param-inputs">
        {showRange && (
          <input
            type="range" aria-label={`${label} slider`} min={def.min} max={def.max} step={step}
            value={Number.isFinite(value) ? Math.min(def.max, Math.max(def.min, value)) : def.default}
            onChange={(e) => onEdit(Number(e.target.value))} onPointerUp={onCommit} onKeyUp={onCommit}
          />
        )}
        <input
          {...common} type="number" step={isInt ? 1 : 'any'} value={Number.isFinite(value) ? value : ''}
          onChange={(e) => onEdit(e.target.value === '' ? Number.NaN : Number(e.target.value))} onBlur={onCommit}
          onKeyDown={(e) => { if (e.key === 'Enter') onCommit(); }}
        />
        {def.unit && <span className="param-unit">{def.unit}</span>}
      </div>
    );
  }
  return (
    <div className={`param${error ? ' has-error' : ''}`} data-param={def.name} title={def.description}>
      <div className="param-head">
        <label htmlFor={id}>{label}</label>
        <span className="param-default">default {fmtParam(def, def.default)}{def.unit ? ` ${def.unit}` : ''}</span>
        <button type="button" className="btn tiny" aria-label={`Reset ${label} to default`} disabled={isDefault(def, value) && !error} onClick={onReset}>Reset</button>
      </div>
      {control}
      {busy && <p className="note" aria-live="polite">Sending...</p>}
      {error && <p id={errId} className="note error" role="alert">{error}</p>}
    </div>
  );
}
