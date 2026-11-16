import { useEffect } from "react";
import { ArrowLeft, ArrowRight, FileQuestion } from "lucide-react";
import { Link, useNavigate } from "react-router-dom";
import { useSiteSettings } from "./site-settings";
import { Button } from "./ui/button";

export function NotFound({
  homeTo = "/",
  homeLabel = "返回首页",
}: {
  homeTo?: string;
  homeLabel?: string;
}) {
  const navigate = useNavigate();
  const { settings } = useSiteSettings();
  useEffect(() => {
    const previousTitle = document.title;
    document.title = `404 · 页面不存在 · ${settings.name}`;
    return () => {
      document.title = previousTitle;
    };
  }, [settings.name]);

  return (
    <section className="not-found-page" aria-labelledby="not-found-title">
      <div className="not-found-symbol" aria-hidden="true">
        <FileQuestion strokeWidth={1.5} />
      </div>
      <p className="not-found-code">404</p>
      <h1 id="not-found-title">页面不存在</h1>
      <p className="not-found-description">
        你访问的页面可能已被移除，或地址输入有误。
      </p>
      <div className="not-found-actions">
        <Button asChild>
          <Link to={homeTo}>
            {homeLabel} <ArrowRight aria-hidden="true" />
          </Link>
        </Button>
        <Button
          variant="outline"
          onClick={() => {
            if (window.history.state?.idx > 0) navigate(-1);
            else navigate(homeTo, { replace: true });
          }}
        >
          <ArrowLeft aria-hidden="true" /> 返回上一页
        </Button>
      </div>
    </section>
  );
}
