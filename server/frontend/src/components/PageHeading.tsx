export function PageHeading({
  eyebrow,
  title,
  description,
  aside,
}: {
  eyebrow: string
  title: string
  description?: string
  aside?: React.ReactNode
}) {
  return (
    <section className="page-heading">
      <div className="page-heading-copy">
        <span className="eyebrow">{eyebrow}</span>
        <h1>{title}</h1>
        {description ? <p>{description}</p> : null}
      </div>
      {aside}
    </section>
  )
}
