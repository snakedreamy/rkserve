import { iconData, type IconNode } from './iconData'

export type IconName = keyof typeof iconData

interface IconProps {
  name: IconName
  size?: number
  className?: string
  strokeWidth?: number
}

// Match lucide-react rendering (1.5 stroke, round caps) without its runtime dependency.
export function Icon({ name, size = 18, className, strokeWidth = 1.5 }: IconProps) {
  const node: IconNode | undefined = iconData[name]
  if (!node) return null
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {node.map(([tag, attrs]) => {
        const { key, ...rest } = attrs
        const Tag = tag as 'path'
        return <Tag key={key} {...rest} />
      })}
    </svg>
  )
}
