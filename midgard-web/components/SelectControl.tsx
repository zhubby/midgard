"use client";

import {
  type KeyboardEvent,
  useEffect,
  useId,
  useRef,
  useState,
} from "react";
import { Check, ChevronDown } from "lucide-react";

export interface SelectOption {
  label: string;
  value: string;
}

interface SelectControlProps {
  ariaLabel: string;
  emptyLabel?: string;
  disabled?: boolean;
  options: SelectOption[];
  value: string;
  onChange: (value: string) => void;
}

export function SelectControl({
  ariaLabel,
  emptyLabel = "No options",
  disabled = false,
  options,
  value,
  onChange,
}: SelectControlProps) {
  const id = useId();
  const rootRef = useRef<HTMLDivElement | null>(null);
  const [open, setOpen] = useState(false);
  const selectedIndex = options.findIndex((option) => option.value === value);
  const selected = selectedIndex >= 0 ? options[selectedIndex] : null;
  const activeIndex = selectedIndex >= 0 ? selectedIndex : 0;

  useEffect(() => {
    if (!open) return;

    function handlePointerDown(event: PointerEvent) {
      if (!rootRef.current?.contains(event.target as Node)) {
        setOpen(false);
      }
    }

    window.addEventListener("pointerdown", handlePointerDown);
    return () => window.removeEventListener("pointerdown", handlePointerDown);
  }, [open]);

  function selectOption(option: SelectOption) {
    onChange(option.value);
    setOpen(false);
  }

  function handleKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
    if (disabled || options.length === 0) return;

    if (event.key === "Escape") {
      setOpen(false);
      return;
    }

    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      if (!open) {
        setOpen(true);
        return;
      }
      selectOption(options[activeIndex]);
      return;
    }

    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
      return;
    }

    event.preventDefault();
    const nextIndex =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? options.length - 1
          : event.key === "ArrowDown"
            ? Math.min(activeIndex + 1, options.length - 1)
            : Math.max(activeIndex - 1, 0);
    onChange(options[nextIndex].value);
    setOpen(true);
  }

  return (
    <div className="select-control" ref={rootRef}>
      <button
        aria-expanded={open}
        aria-haspopup="listbox"
        aria-label={ariaLabel}
        className="select-trigger"
        disabled={disabled || options.length === 0}
        type="button"
        onClick={() => setOpen((current) => !current)}
        onKeyDown={handleKeyDown}
      >
        <span>{selected?.label ?? emptyLabel}</span>
        <ChevronDown aria-hidden="true" />
      </button>

      {open && (
        <div
          aria-label={ariaLabel}
          className="select-menu"
          id={id}
          role="listbox"
        >
          {options.map((option) => {
            const isSelected = option.value === value;
            return (
              <button
                aria-selected={isSelected}
                className={`select-option ${isSelected ? "active" : ""}`}
                key={option.value}
                role="option"
                type="button"
                onClick={() => selectOption(option)}
              >
                <span>{option.label}</span>
                {isSelected && <Check aria-hidden="true" />}
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
