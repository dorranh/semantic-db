import type { ReactNode } from "react";
import clsx from "clsx";
import Heading from "@theme/Heading";
import styles from "./styles.module.css";

type FeatureItem = {
  title: string;
  Svg: React.ComponentType<React.ComponentProps<"svg">>;
  description: ReactNode;
};

const FeatureList: FeatureItem[] = [
  {
    title: "Simple",
    Svg: require("@site/static/img/landing-illustrations/simple.svg").default,
    description: (
      <>
        Query your data in natural language or SQL, with your business context
        built in.
      </>
    ),
  },
  {
    title: "Flexible",
    Svg: require("@site/static/img/landing-illustrations/flexible.svg").default,
    description: (
      <>Query across different files, databases, and data sources.</>
    ),
  },
  {
    title: "Extensible",
    Svg: require("@site/static/img/landing-illustrations/extensible.svg")
      .default,
    description: (
      <>
        Build integrations for your own data sources, including REST and GraphQL
        APIs.
      </>
    ),
  },
];

function Feature({ title, Svg, description }: FeatureItem) {
  return (
    <div className={clsx("col col--4")}>
      <div className="text--center">
        <Svg className={styles.featureSvg} role="img" />
      </div>
      <div className="text--center padding-horiz--md">
        <Heading as="h3">{title}</Heading>
        <p>{description}</p>
      </div>
    </div>
  );
}

export default function HomepageFeatures(): ReactNode {
  return (
    <section className={styles.features}>
      <div className="container">
        <div className="row">
          {FeatureList.map((props, idx) => (
            <Feature key={idx} {...props} />
          ))}
        </div>
      </div>
    </section>
  );
}
