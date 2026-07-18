import { Github } from 'lucide-react'

export function ProjectFooter() {
  return (
    <footer className="project-footer">
      <a
        aria-label="AsukaCC/LightMonitor GitHub repository"
        href="https://github.com/AsukaCC/LightMonitor"
        rel="noreferrer"
        target="_blank"
      >
        <Github aria-hidden="true" size={16} />
        <span>AsukaCC/LightMonitor</span>
      </a>
    </footer>
  )
}
