// `@dgos/ui` (D066). Phase 1 exports the appearance kernel; Phase 4 adds the controls and display components;
// Phase 5 adds Modal, Tooltip, Toast, the status icons, error mapping and ErrorPage. Phase 6 injects a subset of
// these into modules as `host.ui` (src/modules/moduleHostUi.ts).
export { cn } from './lib/cn';
export { Button } from './components/Button';
export type { ButtonProps, ButtonSize, ButtonVariant } from './components/Button';
export { Card } from './components/Card';
export { Checkbox } from './components/Checkbox';
export type { CheckboxProps } from './components/Checkbox';
export { Divider } from './components/Divider';
export type { DividerOrientation, DividerProps } from './components/Divider';
export { Input } from './components/Input';
export type { InputProps, InputType, MultilineInputProps, SingleLineInputProps } from './components/Input';
export { Loading } from './components/Loading';
export type { LoadingProps, LoadingSize } from './components/Loading';
export { RadioGroup } from './components/RadioGroup';
export type { RadioGroupProps, RadioOption } from './components/RadioGroup';
export { Select } from './components/Select';
export type { SelectOption, SelectProps } from './components/Select';
export { Stack } from './components/Stack';
export type { StackDirection, StackGap, StackProps } from './components/Stack';
export { Switch } from './components/Switch';
export type { SwitchProps } from './components/Switch';
export { Tabs } from './components/Tabs';
export type { TabsProps } from './components/Tabs';
export {
  DEFAULTS,
  DENSITIES,
  THEMES,
  createAppearanceController,
  disposeAppearance,
  initAppearance,
  useAppearance,
} from './appearance';
export type {
  Appearance,
  AppearanceController,
  AppearanceDeps,
  Density,
  ResolvedTheme,
  Theme,
} from './appearance';
export { ErrorBoundary, ErrorPage } from './components/ErrorPage';
export type { ErrorPageKind, ErrorPageProps } from './components/ErrorPage';
export { CONFIRM_FAILED_TEXT, Modal, ModalHost } from './components/Modal';
export type { ModalProps } from './components/Modal';
export type { AlertOptions, ConfirmOptions, ModalHandle, ModalIcon, ModalKind } from './feedback/modalStore';
export { StatusIcon } from './components/StatusIcon';
export type { StatusIconName, StatusIconProps } from './components/StatusIcon';
export { Tooltip } from './components/Tooltip';
export type { TooltipProps } from './components/Tooltip';
export { ToastViewport } from './components/ToastViewport';
export { clearToasts, toast } from './feedback/toastStore';
export type { ToastAction, ToastOptions, ToastType } from './feedback/toastStore';
export { ERROR_MESSAGES, lookupError } from './errors/messages';
export type { ErrorMessage, Severity } from './errors/messages';
export { extractErrorCode, handleError } from './errors/handleError';
export type { HandleErrorOptions } from './errors/handleError';
export {
  EmptyState,
  FilterBar,
  FixedActionBar,
  Notice,
  ResourceCard,
  ResourceDetail,
  ResourceLayout,
  ResourceList,
  SearchField,
  SectionHeader,
  StatusBadge,
} from './components/PagePatterns';
export type {
  EmptyStateProps,
  FilterBarProps,
  FixedActionBarProps,
  NoticeProps,
  NoticeTone,
  ResourceCardProps,
  ResourceDetailProps,
  ResourceLayoutProps,
  ResourceListProps,
  SearchFieldProps,
  SectionHeaderProps,
  StatusBadgeProps,
  StatusBadgeStatus,
} from './components/PagePatterns';
export { resourceView } from '../resourceState';
export type { ResourceReadState, ResourceRefresh, ResourceViewState } from '../resourceState';
export { Icon } from './components/Icon';
export type { IconProps } from './components/Icon';
export { Popover } from './components/Popover';
export type { PopoverProps } from './components/Popover';
export { ContextMenu, DropdownMenu } from './components/Menu';
export type { MenuItem, ContextMenuProps, DropdownMenuProps } from './components/Menu';
export { Slider } from './components/Slider';
export type { SliderProps } from './components/Slider';
