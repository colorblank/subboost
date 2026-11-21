import { MetadataRoute } from "next";
import { createSubBoostManifest } from "@subboost/ui/brand";

export const dynamic = "force-static";

export default function manifest(): MetadataRoute.Manifest {
  return createSubBoostManifest();
}
