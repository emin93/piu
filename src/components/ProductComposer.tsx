import { useId, type FormEvent, type KeyboardEvent, type ReactNode, type Ref } from "react";

import { Textarea } from "@/components/ui/textarea";

interface ProductComposerError {
  action?: ReactNode;
  message: ReactNode;
}

export interface ProductComposerProps {
  actions?: ReactNode;
  ariaDescribedBy?: string;
  ariaLabel: string;
  error?: ProductComposerError;
  inputRef?: Ref<HTMLTextAreaElement>;
  layout: "centered" | "docked";
  onSubmit?: () => void;
  onValueChange?: (value: string) => void;
  placeholder?: string;
  readOnly?: boolean;
  status?: ReactNode;
  submitOnMetaEnter?: boolean;
  value: string;
}

export function ProductComposer({
  actions,
  ariaDescribedBy,
  ariaLabel,
  error,
  inputRef,
  layout,
  onSubmit,
  onValueChange,
  placeholder,
  readOnly = false,
  status,
  submitOnMetaEnter = false,
  value,
}: ProductComposerProps) {
  const generatedId = useId();
  const errorId = error ? `${generatedId}-error` : undefined;
  const describedBy = [ariaDescribedBy, errorId].filter(Boolean).join(" ") || undefined;
  const submit = (event: FormEvent) => {
    event.preventDefault();
    onSubmit?.();
  };
  const submitShortcut = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (!submitOnMetaEnter || event.key !== "Enter" || !event.metaKey || !onSubmit) return;
    event.preventDefault();
    onSubmit();
  };
  const content = (
    <>
      <Textarea
        aria-describedby={describedBy}
        aria-label={ariaLabel}
        className="product-composer-input"
        onChange={onValueChange ? (event) => onValueChange(event.target.value) : undefined}
        onKeyDown={submitShortcut}
        placeholder={placeholder}
        readOnly={readOnly}
        ref={inputRef}
        rows={layout === "centered" ? 4 : 3}
        value={value}
      />
      <div className="product-composer-footer">
        <div className="product-composer-status">{status}</div>
        <div className="product-composer-actions">{actions}</div>
      </div>
      {error ? (
        <div className="product-composer-error" role="alert">
          <div className="product-composer-error-copy" id={errorId}>
            {error.message}
          </div>
          {error.action}
        </div>
      ) : null}
    </>
  );

  if (readOnly) {
    return (
      <div className="product-composer" data-composer-layout={layout} data-composer-readonly="true">
        {content}
      </div>
    );
  }

  return (
    <form className="product-composer" data-composer-layout={layout} onSubmit={submit}>
      {content}
    </form>
  );
}
